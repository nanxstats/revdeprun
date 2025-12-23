# Monkey patches for the {xfun} download helper.
#
# Motivation:
# - xfun:::download_tarball() downloads tarballs serially, which slows down
#   revdep runs with large reverse dependency sets. This patch parallelizes
#   the download loop using forked workers.

xfun_patch_parallel_download <- function() {
  if (!requireNamespace("xfun", quietly = TRUE)) {
    stop("xfun must be installed.", call. = FALSE)
  }

  ns <- asNamespace("xfun")

  old <- list(
    download_tarball = get("download_tarball", envir = ns)
  )

  patched_download_tarball <- function(
    pkgs,
    db = available.packages(type = "source"),
    dir = ".",
    retry = 3
  ) {
    if (!dir_exists(dir)) dir.create(dir, recursive = TRUE)
    pkgs <- as.character(pkgs)
    pkgs <- pkgs[!is.na(pkgs) & nzchar(pkgs)]
    if (!length(pkgs)) {
      return(character())
    }

    z <- file.path(dir, sprintf("%s_%s.tar.gz", pkgs, db[pkgs, "Version"]))
    parallel::mcmapply(
      function(p, z) {
        # remove other versions of the package tarball
        unlink(setdiff(list.files(dir, sprintf("^%s_.+.tar.gz", p), full.names = TRUE), z))
        for (i in seq_len(retry)) {
          if (file_exists(z)) break
          try(
            download.file(paste(db[p, "Repository"], basename(z), sep = "/"), z, mode = "wb"),
            silent = TRUE
          )
        }
      },
      pkgs,
      z,
      SIMPLIFY = FALSE,
      mc.cores = getOption("xfun.rev_check.download_cores", 1L)
    )
    z
  }

  environment(patched_download_tarball) <- ns

  assignInNamespace("download_tarball", patched_download_tarball, ns = "xfun")

  class(old) <- c("xfun_parallel_download_patch", class(old))
  invisible(old)
}

xfun_unpatch_parallel_download <- function(patch) {
  if (!inherits(patch, "xfun_parallel_download_patch")) {
    stop("Not an xfun patch object.", call. = FALSE)
  }
  assignInNamespace("download_tarball", patch$download_tarball, ns = "xfun")
  invisible(TRUE)
}
