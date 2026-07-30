# Monkey patches for the {pkgdepends} install scheduler and P3M downloads
#
# Motivation:
# - pkgdepends::install_package_plan() starts only one new task per event-loop
#   iteration, which can under-fill the worker pool when many installs finish
#   between polls (common with small binary packages).
# - stop_task_install() updates deps_left for *all* packages on every install,
#   which can be costly for very large plans even when only source builds need it.
# - Posit Public Package Manager (P3M) limits clients to roughly 2,000 requests
#   per five minutes. Both pak and xfun can exceed this during large revchecks.

P3M_REQUEST_LIMIT <- 1800L
P3M_WINDOW_SECONDS <- 5 * 60 + 5

p3m_rate_limit_is_url <- function(urls) {
  any(grepl(
    "^https?://packagemanager[.]posit[.]co(/|$)",
    urls,
    ignore.case = TRUE
  ), na.rm = TRUE)
}

p3m_rate_limit_default_state <- function() {
  list(used = 0L, last_completed = NA_real_)
}

p3m_rate_limit_read_state <- function(state_file) {
  state <- tryCatch(readRDS(state_file), error = function(err) NULL)
  if (
    !is.list(state) ||
      length(state$used) != 1L ||
      length(state$last_completed) != 1L ||
      !is.numeric(state$used) ||
      !is.numeric(state$last_completed) ||
      is.na(state$used) ||
      state$used < 0L ||
      state$used > P3M_REQUEST_LIMIT
  ) {
    return(p3m_rate_limit_default_state())
  }

  state$used <- as.integer(state$used)
  state$last_completed <- as.numeric(state$last_completed)
  age <- as.numeric(Sys.time()) - state$last_completed
  if (!is.finite(age) || age >= P3M_WINDOW_SECONDS) {
    p3m_rate_limit_default_state()
  } else {
    state
  }
}

p3m_rate_limit_write_state <- function(state_file, state) {
  saveRDS(state, state_file)
  invisible(state)
}

p3m_rate_limit_wait_seconds <- function(state) {
  if (state$used < P3M_REQUEST_LIMIT) {
    return(0)
  }
  max(
    0,
    P3M_WINDOW_SECONDS - (as.numeric(Sys.time()) - state$last_completed)
  )
}

p3m_rate_limit_reset <- function(state_file) {
  p3m_rate_limit_write_state(state_file, p3m_rate_limit_default_state())
}

p3m_rate_limit_record <- function(state_file, state, requests) {
  state$used <- as.integer(state$used + requests)
  state$last_completed <- as.numeric(Sys.time())
  p3m_rate_limit_write_state(state_file, state)
}

pkgdepends_patch_parallel_install <- function(p3m_state_file) {
  if (!requireNamespace("pkgdepends", quietly = TRUE)) {
    stop("pkgdepends must be installed.", call. = FALSE)
  }

  p3m_state_file <- normalizePath(p3m_state_file, mustWork = TRUE)
  ns <- asNamespace("pkgdepends")

  old <- list(
    install_package_plan = get("install_package_plan", envir = ns),
    stop_task_install = get("stop_task_install", envir = ns),
    stop_task_build = get("stop_task_build", envir = ns),
    pkgplan_async_download_internal = get(
      "pkgplan_async_download_internal",
      envir = ns
    )
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
  patch_env$p3m_state_file <- p3m_state_file
  patch_env$p3m_rate_limit_is_url <- p3m_rate_limit_is_url
  patch_env$p3m_rate_limit_read_state <- p3m_rate_limit_read_state
  patch_env$p3m_rate_limit_write_state <- p3m_rate_limit_write_state
  patch_env$p3m_rate_limit_wait_seconds <- p3m_rate_limit_wait_seconds
  patch_env$p3m_rate_limit_reset <- p3m_rate_limit_reset
  patch_env$p3m_rate_limit_record <- p3m_rate_limit_record
  patch_env$P3M_REQUEST_LIMIT <- P3M_REQUEST_LIMIT
  patch_env$deps_left_remove_if_needed <- function(state, pkg) {
    need <- which(!state$plan$package_done | !state$plan$build_done)
    if (length(need)) {
      state$plan$deps_left[need] <- lapply(state$plan$deps_left[need], setdiff, pkg)
    }
    state
  }

  patched_pkgplan_async_download_internal <- function(
    self,
    private,
    what,
    which
  ) {
    if (any(what$status != "OK")) {
      stop("Resolution has errors, cannot start downloading")
    }
    start <- Sys.time()
    private$progress_bar <- private$create_progress_bar(what)

    is_p3m <- vapply(
      what$sources,
      p3m_rate_limit_is_url,
      FUN.VALUE = logical(1)
    )
    pending <- seq_len(nrow(what))
    downloads <- vector("list", nrow(what))

    download_one <- function(idx) {
      force(idx)
      private$download_res(
        what[idx, ],
        which = which,
        on_progress = function(data) {
          private$update_progress_bar(idx, "got", data)
        }
      )$then(function(value) {
        private$update_progress_bar(idx, "done", value)
        value
      })$catch(
        error = function(err) private$update_progress_bar(idx, "error", err)
      )
    }

    run_next_batch <- NULL
    run_next_batch <- function() {
      if (!length(pending)) {
        return(async_constant(downloads))
      }

      state <- p3m_rate_limit_read_state(p3m_state_file)
      p3m_pending <- pending[is_p3m[pending]]
      other_pending <- pending[!is_p3m[pending]]
      capacity <- P3M_REQUEST_LIMIT - state$used

      if (length(p3m_pending) && capacity == 0L) {
        wait <- p3m_rate_limit_wait_seconds(state)
        cli::cli_alert_info(sprintf(
          "P3M request budget reached; resuming downloads in %.0f seconds.",
          ceiling(wait)
        ))
        return(
          # pkgdepends wraps pkgcache's private delay() as async_delay().
          async_delay(wait)$then(function(value) {
            p3m_rate_limit_reset(p3m_state_file)
            run_next_batch()
          })
        )
      }

      selected_p3m <- head(p3m_pending, capacity)
      selected <- c(other_pending, selected_p3m)
      pending <<- setdiff(pending, selected)

      when_all(.list = lapply(selected, download_one))$then(function(values) {
        downloads[selected] <<- values
        if (length(selected_p3m)) {
          p3m_rate_limit_record(
            p3m_state_file,
            state,
            length(selected_p3m)
          )
        }
        run_next_batch()
      })
    }

    run_next_batch()$then(function(dls) {
      what$fulltarget <- vcapply(dls, "[[", "fulltarget")
      what$fulltarget_tree <- vcapply(dls, "[[", "fulltarget_tree")
      what$download_status <- vcapply(dls, "[[", "download_status")
      what$download_error <- lapply(dls, function(x) x$download_error[[1]])
      what$file_size <- vdapply(dls, "[[", "file_size")
      what$used_cached_binary <- vlapply(dls, "[[", "used_cached_binary")
      class(what) <- c("pkgplan_downloads", class(what))
      attr(what, "metadata")$download_start <- start
      attr(what, "metadata")$download_end <- Sys.time()
      what
    })$finally(function() private$done_progress_bar())
  }

  environment(patched_pkgplan_async_download_internal) <- patch_env

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
  assignInNamespace(
    "pkgplan_async_download_internal",
    patched_pkgplan_async_download_internal,
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
  assignInNamespace(
    "pkgplan_async_download_internal",
    patch$pkgplan_async_download_internal,
    ns = "pkgdepends"
  )
  assignInNamespace("stop_task_install", patch$stop_task_install, ns = "pkgdepends")
  assignInNamespace("stop_task_build", patch$stop_task_build, ns = "pkgdepends")
  invisible(TRUE)
}

pak_patch_parallel_install <- function(patch_file, p3m_state_file) {
  if (!requireNamespace("pak", quietly = TRUE)) {
    stop("pak must be installed.", call. = FALSE)
  }
  patch_file <- normalizePath(patch_file, mustWork = TRUE)
  p3m_state_file <- normalizePath(p3m_state_file, mustWork = TRUE)
  pak:::remote(
    function(patch_file, p3m_state_file) {
      source(patch_file, local = TRUE)
      pkgdepends_patch_parallel_install(p3m_state_file)
      TRUE
    },
    list(
      patch_file = patch_file,
      p3m_state_file = p3m_state_file
    )
  )
  invisible(TRUE)
}

xfun_patch_p3m_downloads <- function(p3m_state_file) {
  if (!requireNamespace("xfun", quietly = TRUE)) {
    stop("xfun must be installed.", call. = FALSE)
  }

  p3m_state_file <- normalizePath(p3m_state_file, mustWork = TRUE)
  ns <- asNamespace("xfun")
  original <- get("download_tarball", envir = ns)
  if (inherits(original, "revdeprun_p3m_patch")) {
    return(invisible(original))
  }

  patch_env <- new.env(parent = ns)
  patch_env$original_download_tarball <- original
  patch_env$p3m_state_file <- p3m_state_file
  patch_env$p3m_rate_limit_is_url <- p3m_rate_limit_is_url
  patch_env$p3m_rate_limit_read_state <- p3m_rate_limit_read_state
  patch_env$p3m_rate_limit_wait_seconds <- p3m_rate_limit_wait_seconds
  patch_env$p3m_rate_limit_reset <- p3m_rate_limit_reset
  patch_env$p3m_rate_limit_record <- p3m_rate_limit_record
  patch_env$P3M_REQUEST_LIMIT <- P3M_REQUEST_LIMIT

  patched_download_tarball <- function(
    p,
    db = available.packages(type = "source"),
    dir = ".",
    retry = 3
  ) {
    if (!length(p)) {
      return(original_download_tarball(p, db, dir, retry))
    }

    repositories <- db[p, "Repository"]
    expected <- file.path(
      dir,
      sprintf("%s_%s.tar.gz", p, db[p, "Version"])
    )
    is_p3m <- vapply(
      repositories,
      p3m_rate_limit_is_url,
      FUN.VALUE = logical(1)
    ) & !file.exists(expected)
    pending <- seq_along(p)
    tarballs <- character(length(p))

    while (length(pending)) {
      state <- p3m_rate_limit_read_state(p3m_state_file)
      p3m_pending <- pending[is_p3m[pending]]
      other_pending <- pending[!is_p3m[pending]]
      capacity <- P3M_REQUEST_LIMIT - state$used

      if (length(p3m_pending) && capacity == 0L) {
        wait <- p3m_rate_limit_wait_seconds(state)
        message(sprintf(
          "P3M request budget reached; resuming downloads in %.0f seconds.",
          ceiling(wait)
        ))
        Sys.sleep(wait)
        p3m_rate_limit_reset(p3m_state_file)
        next
      }

      selected_p3m <- head(p3m_pending, capacity)
      selected <- c(other_pending, selected_p3m)
      pending <- setdiff(pending, selected)
      tarballs[selected] <- original_download_tarball(
        p[selected],
        db,
        dir,
        retry
      )

      if (length(selected_p3m)) {
        p3m_rate_limit_record(
          p3m_state_file,
          state,
          length(selected_p3m)
        )
      }
    }

    tarballs
  }

  environment(patched_download_tarball) <- patch_env
  class(patched_download_tarball) <- c(
    "revdeprun_p3m_patch",
    class(patched_download_tarball)
  )
  assignInNamespace(
    "download_tarball",
    patched_download_tarball,
    ns = "xfun"
  )
  invisible(original)
}
