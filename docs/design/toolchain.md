---
icon: lucide/wrench
---

# Toolchain

revdeprun provisions just enough of a toolchain to make `R CMD check` work
across a wide range of packages.

## R version resolution

R versions are resolved via the R-hub R versions API.

This supports the `r-lib/actions/setup-r` style shorthands like:

- `release` (default)
- `devel`
- `oldrel-1`
- explicit versions like `4.3.3`

See `src/r_version.rs`.

## R installation

On Ubuntu, revdeprun downloads the platform-specific `.deb`, installs
prerequisites, and installs the package with `gdebi`. It then creates stable
symlinks in `/usr/local/bin` so that `R` and `Rscript` are on `PATH`.

See `src/r_install.rs`.

If you already have R, use `--skip-r-install` to reuse it.

## Quarto, pandoc, TinyTeX

Many packages build vignettes. That means you need tooling.

revdeprun will install:

- Quarto (if missing), pinned to the latest stable release at runtime.
- pandoc (via `apt`).
- TinyTeX (via `quarto install tinytex`) and symlinks for common TeX binaries.

The goal here is not a perfect TeX setup. Instead, the goal is
"enough that `R CMD check` doesn't fail for boring reasons".
