---
icon: lucide/layers
---

# Dependencies

Reverse dependency checking is won or lost in dependency installation.

The core strategy in revdeprun is:

- Install a curated set of dependencies *once* into an isolated library.
- Prefer binaries wherever possible.
- Keep going in the presence of missing or broken packages.

All of this is implemented in `src/revdep.rs` as generated R scripts.

## Binary-first repositories

On Ubuntu, revdeprun configures repositories like this:

- CRAN binaries from Posit Public Package Manager (P3M), keyed by Ubuntu codename.
- CRAN source and metadata from P3M `cran/latest`.
- Bioconductor via P3M.

This gives you fast installs without giving up accurate dependency metadata.

## The minimal dependency set

In theory, the "true" dependency set of a huge reverse dependency list
is enormous. In practice, you want a set that is:

- Big enough that `R CMD check` succeeds for all packages.
- Small enough that you are not installing CRAN for fun.

revdeprun approximates this by installing:

- Your package and dependencies of your package (from both the CRAN metadata
  and your `DESCRIPTION` file).
- All reverse dependencies of your package (via `Depends`/`Imports`/`LinkingTo`/`Suggests`).
- Dependencies for running `R CMD check` on all reverse dependencies.
  This includes the full recursive closure of their hard dependencies,
  but only the first-order `Suggests`, not the full recursive `Suggests` closure.

This is a deliberate compromise: `Suggests` matter for checks, but recursive
`Suggests` is too much.

## Reliability tactics

The install script:

- Adds `?ignore-build-errors&ignore-unavailable` to install targets, so one
  failing package doesn't take down the entire run.
- Retries `pak::pkg_install()` failures (network blips happen).
- Installs into `revdep/library/` by setting `R_LIBS_USER` and `.libPaths()`.
