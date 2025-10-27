# revdeprun

[![crates.io version](https://img.shields.io/crates/v/revdeprun)](https://crates.io/crates/revdeprun)
[![CI tests](https://github.com/nanxstats/revdeprun/actions/workflows/ci.yml/badge.svg)](https://github.com/nanxstats/revdeprun/actions/workflows/ci.yml)
![License](https://img.shields.io/crates/l/revdeprun)

Easy reverse dependency checks for R via revdepcheck with cloud-ready environment setup.

## Installation

Install system dependencies for building Rust crates and building R package
dependencies of revdepcheck:

```bash
sudo apt-get update
sudo apt-get install -y build-essential libssl-dev libcurl4-openssl-dev
```

Install Rust with rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Install `revdeprun` from crates.io using Cargo:

```bash
cargo install revdeprun
```

To try the latest development version directly from GitHub:

```bash
cargo install --git https://github.com/nanxstats/revdeprun.git
```

## Requirements

- Ubuntu 20.04 or newer.
- Git available on `PATH`.
- Network access to download R, R packages, and the Git repository.
- `sudo` access for `apt-get`, `gdebi`, and system-wide `/opt/R` installation.

Running inside a fresh, one-off cloud instance is strongly recommended because
reverse dependency checks execute third-party code.

## Usage

The CLI provisions the R toolchain, sets the package repository,
and runs revdepcheck:

```bash
revdeprun https://github.com/YOUR-USERNAME/YOUR-REPOSITORY.git
```

By default, the current release version of R for Ubuntu is installed
and the number of workers is set to use all available CPU cores.

### Command options

```
$ revdeprun --help
Provision R and run revdepcheck end-to-end

Usage: revdeprun [OPTIONS] <REPOSITORY>

Arguments:
  <REPOSITORY>  Git URL or filesystem path pointing to the target R package repository

Options:
      --r-version <R_VERSION>  R version specification to install (e.g. release, 4.3.3, oldrel-1) [default: release]
      --num-workers <N>        Number of parallel workers to pass to revdepcheck
      --work-dir <WORK_DIR>    Optional workspace directory where temporary files are created
      --skip-r-install         Skip installing R and reuse the system-wide installation
  -h, --help                   Print help
  -V, --version                Print version
```

### Typical workflow

1. Provision a clean Ubuntu VM with sufficient CPU and memory.
2. Install `revdeprun` (for example, `cargo install revdeprun`).
3. Run `revdeprun <repo>` with optional flags, such as:
   - `revdeprun --r-version release https://github.com/YOUR-USERNAME/YOUR-REPOSITORY.git`
   - `revdeprun --num-workers 48 --work-dir /data/workspace git@github.com:YOUR-USERNAME/YOUR-REPOSITORY.git`
4. Review the results under `<repo>/revdep`.

### Notes

- `revdeprun` installs R into `/opt/R/...` and symlinks binaries to `/usr/local/bin`.
- The tool uses the latest development version of revdepcheck from GitHub.
- If you already provision R and required packages, pass `--skip-r-install`.
- To point at a local checkout instead of cloning, supply the path directly:
  `revdeprun ~/workspace/YOUR-REPOSITORY`.
