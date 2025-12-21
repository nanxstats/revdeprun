# Monkey patches for the {pkgdepends} install scheduler
#
# Motivation:
# - pkgdepends::install_package_plan() starts only one new task per event-loop
#   iteration, which can under-fill the worker pool when many installs finish
#   between polls (common with small binary packages).
# - stop_task_install() updates deps_left for *all* packages on every install,
#   which can be costly for very large plans even when only source builds need it.

pkgdepends_patch_parallel_install <- function() {
  if (!requireNamespace("pkgdepends", quietly = TRUE)) {
    stop("pkgdepends must be installed.", call. = FALSE)
  }

  ns <- asNamespace("pkgdepends")

  old <- list(
    install_package_plan = get("install_package_plan", envir = ns),
    stop_task_install = get("stop_task_install", envir = ns),
    stop_task_build = get("stop_task_build", envir = ns)
  )

  patched_install_package_plan <- function(
    plan,
    lib = .libPaths()[[1]],
    num_workers = 1,
    cache = NULL
  ) {
    start <- Sys.time()
    cli::ansi_hide_cursor()
    on.exit(cli::ansi_show_cursor())

    cli::cli_div(
      theme = list(
        ".timestamp" = list(
          color = "darkgrey",
          before = "(",
          after = ")"
        )
      )
    )

    required_columns <- c(
      "type",
      "binary",
      "dependencies",
      "file",
      "needscompilation",
      "package"
    )
    assert_that(
      inherits(plan, "data.frame"),
      all(required_columns %in% colnames(plan)),
      is_string(lib),
      is_count(num_workers, min = 1L)
    )

    if (!"vignettes" %in% colnames(plan)) plan$vignettes <- FALSE
    if (!"metadata" %in% colnames(plan)) {
      plan$metadata <- replicate(nrow(plan), character(), simplify = FALSE)
    }
    if (!"packaged" %in% colnames(plan)) plan$packaged <- TRUE
    if (!"used_cached_binary" %in% colnames(plan)) {
      plan$used_cached_binary <- FALSE
    }

    plan <- add_recursive_dependencies(plan)

    config <- list(
      lib = lib,
      num_workers = num_workers,
      show_time = tolower(Sys.getenv("PKG_OMIT_TIMES")) != "true"
    )
    state <- make_start_state(plan, config)
    state$cache <- cache
    state$progress <- create_progress_bar(state)
    on.exit(done_progress_bar(state), add = TRUE)

    withCallingHandlers(
      {
        for (i in seq_len(state$config$num_workers)) {
          task <- select_next_task(state)
          state <- start_task(state, task)
        }

        repeat {
          if (are_we_done(state)) break
          update_progress_bar(state)

          events <- poll_workers(state)
          state <- handle_events(state, events)

          # Key change: refill the worker pool, not just one task.
          while (length(state$workers) < state$config$num_workers) {
            task <- select_next_task(state)
            if (identical(task$name, "idle")) break
            state <- start_task(state, task)
          }
        }
      },
      error = function(e) kill_all_processes(state)
    )

    create_install_result(state)
  }

  environment(patched_install_package_plan) <- ns

  # Put helpers into a dedicated environment so patched functions can find
  # them even if we set their environment to the pkgdepends namespace.
  patch_env <- new.env(parent = ns)
  patch_env$deps_left_remove_if_needed <- function(state, pkg) {
    need <- which(!state$plan$package_done | !state$plan$build_done)
    if (length(need)) {
      state$plan$deps_left[need] <- lapply(state$plan$deps_left[need], setdiff, pkg)
    }
    state
  }

  patched_stop_task_install <- function(state, worker) {
    success <- worker$process$get_exit_status() == 0
    pkgidx <- worker$task$args$pkgidx
    pkg <- state$plan$package[pkgidx]
    version <- state$plan$version[pkgidx]
    time <- Sys.time() - state$plan$install_time[[pkgidx]]
    ptime <- format_time$pretty_sec(as.numeric(time, units = "secs"))
    note <- installed_note(state$plan[pkgidx, ])

    if (success) {
      alert(
        "success",
        paste0(
          "Installed {.pkg {pkg}} {.version {version}} {note}",
          if (isTRUE(state$config$show_time)) " {.timestamp {ptime}}"
        )
      )
    } else {
      alert("danger", "Failed to install {.pkg {pkg}} {.version {version}}")
    }
    update_progress_bar(state, 1L)

    state$plan$install_done[[pkgidx]] <- TRUE
    state$plan$install_time[[pkgidx]] <- time
    state$plan$install_error[[pkgidx]] <- !success
    state$plan$install_stdout[[pkgidx]] <- worker$stdout
    state$plan$worker_id[[pkgidx]] <- NA_character_

    if (!success) {
      throw(pkg_error(
        "Failed to install binary package {.pkg {pkg}}.",
        .class = "package_install_error"
      ))
    }

    deps_left_remove_if_needed(state, pkg)
  }

  environment(patched_stop_task_install) <- patch_env

  patched_stop_task_build <- function(state, worker) {
    success <- worker$process$get_exit_status() == 0

    pkgidx <- worker$task$args$pkgidx
    pkg <- state$plan$package[pkgidx]
    version <- state$plan$version[pkgidx]
    time <- Sys.time() - state$plan$build_time[[pkgidx]]
    ptime <- format_time$pretty_sec(as.numeric(time, units = "secs"))
    prms <- state$plan$params[[pkgidx]]

    if (success) {
      alert(
        "success",
        paste0(
          "Built {.pkg {pkg}} {.version {version}}",
          if (isTRUE(state$config$show_time)) " {.timestamp {ptime}}"
        )
      )
      state$plan$file[pkgidx] <- worker$process$get_built_file()
    } else {
      ignore_error <- is_true_param(prms, "ignore-build-errors")
      alert(
        if (ignore_error) "warning" else "danger",
        paste0(
          "Failed to build {.pkg {pkg}} {.version {version}}",
          if (isTRUE(state$config$show_time)) " {.timestamp {ptime}}"
        )
      )
    }
    update_progress_bar(state, 1L)

    state$plan$build_done[[pkgidx]] <- TRUE
    state$plan$build_time[[pkgidx]] <- time
    state$plan$build_stdout[[pkgidx]] <- worker$stdout
    state$plan$worker_id[[pkgidx]] <- NA_character_

    if (success) {
      state$plan$build_error[[pkgidx]] <- FALSE
    } else {
      build_error <- list(
        package = pkg,
        version = version,
        stdout = worker$stdout,
        time = time
      )
      state$plan$build_error[[pkgidx]] <- build_error

      ignore_error <- is_true_param(prms, "ignore-build-errors")
      if (ignore_error) {
        state$plan$install_done[[pkgidx]] <- TRUE
        state <- deps_left_remove_if_needed(state, pkg)
      } else {
        throw(pkg_error(
          "Failed to build source package {.pkg {pkg}}.",
          .data = build_error,
          .class = "package_build_error"
        ))
      }
    }

    if (success && !is.null(state$cache) && !is_true_param(prms, "nocache")) {
      ptfm <- current_r_platform()
      rv <- current_r_version()
      target <- paste0(state$plan$target[pkgidx], "-", ptfm, "-", rv)
      tryCatch(
        state$cache$add(
          state$plan$file[pkgidx],
          target,
          package = pkg,
          version = version,
          built = TRUE,
          sha256 = state$plan$extra[[pkgidx]]$remotesha,
          vignettes = state$plan$vignettes[pkgidx],
          platform = ptfm,
          rversion = rv
        ),
        error = function(err) {
          alert(
            "warning",
            "Failed to add {.pkg {pkg}} \\
               {.version {version}} ({ptfm}) to the cache"
          )
        }
      )
    }

    state
  }

  environment(patched_stop_task_build) <- patch_env

  assignInNamespace(
    "install_package_plan",
    patched_install_package_plan,
    ns = "pkgdepends"
  )
  assignInNamespace("stop_task_install", patched_stop_task_install, ns = "pkgdepends")
  assignInNamespace("stop_task_build", patched_stop_task_build, ns = "pkgdepends")

  class(old) <- c("pkgdepends_parallel_patch", class(old))
  old
}

pkgdepends_unpatch_parallel_install <- function(patch) {
  if (!inherits(patch, "pkgdepends_parallel_patch")) {
    stop("Not a pkgdepends patch object.", call. = FALSE)
  }
  assignInNamespace("install_package_plan", patch$install_package_plan, ns = "pkgdepends")
  assignInNamespace("stop_task_install", patch$stop_task_install, ns = "pkgdepends")
  assignInNamespace("stop_task_build", patch$stop_task_build, ns = "pkgdepends")
  invisible(TRUE)
}

pak_patch_parallel_install <- function(patch_file) {
  if (!requireNamespace("pak", quietly = TRUE)) {
    stop("pak must be installed.", call. = FALSE)
  }
  patch_file <- normalizePath(patch_file, mustWork = TRUE)
  pak:::remote(
    function(patch_file) {
      source(patch_file, local = TRUE)
      pkgdepends_patch_parallel_install()
      TRUE
    },
    list(patch_file = patch_file)
  )
  invisible(TRUE)
}
