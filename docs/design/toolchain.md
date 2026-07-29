---
icon: lucide/wrench
---

# Toolchain

revdeprun provisions just enough of a toolchain to make `R CMD check` work
across a wide range of packages.

## R version resolution

R versions are resolved via the R-hub R versions API unless
`--skip-r-install` is set.

This supports the `r-lib/actions/setup-r` style shorthands like:

- `release` (default)
- `devel`
- `oldrel-1`
- explicit versions like `4.3.3`

See `src/r_version.rs`.

Debian remains an off-label environment. When the API does not recognize a
future Debian release, revdeprun retries successively older Debian releases
until the API returns a compatible installer. Other API errors, such as an
invalid R version specification, are returned immediately.

## R installation

On Ubuntu, revdeprun downloads the platform-specific `.deb`, installs
prerequisites, and installs the package with `gdebi`. It then creates stable
symlinks in `/usr/local/bin` so that `R` and `Rscript` are on `PATH`.

See `src/r_install.rs`.

If you already have R, use `--skip-r-install` to reuse it. This bypasses both
version resolution and installation, so `--r-version` is ignored and the
system-wide `R` and `Rscript` commands must already be on `PATH`. It also
bypasses the document toolchain provisioning described below.

## Quarto, pandoc, TinyTeX

Many packages build vignettes. That means you need tooling.

When R installation is not skipped, revdeprun will install:

- Quarto (if missing), pinned to the latest stable release at runtime.
- pandoc (via `apt`).
- TinyTeX (via `quarto install tinytex`) and symlinks for common TeX binaries.

The goal here is not a perfect TeX setup. Instead, the goal is
"enough that `R CMD check` doesn't fail for boring reasons".
