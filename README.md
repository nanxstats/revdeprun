# revdeprun

[![crates.io version](https://img.shields.io/crates/v/revdeprun)](https://crates.io/crates/revdeprun)
[![CI tests](https://github.com/nanxstats/revdeprun/actions/workflows/ci.yml/badge.svg)](https://github.com/nanxstats/revdeprun/actions/workflows/ci.yml)
![License](https://img.shields.io/crates/l/revdeprun)

A command-line tool that automates reverse dependency checking for R packages.
Provision R on Ubuntu, install system dependencies, preinstall revdep
dependency binaries, configure environment context,
and run `xfun::rev_check()` in a single command.
Designed for cloud environments where you need reproducible, isolated test
runs without tedious manual setup.

## Installation

### Prerequisites

Install Rust:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Install C compiler and linker:

```bash
sudo apt-get update && sudo apt-get install -y build-essential
```

### Install revdeprun

From crates.io (stable release):

```bash
cargo install revdeprun
```

From GitHub (latest development version):

```bash
cargo install --git https://github.com/nanxstats/revdeprun.git
```

**Note**: If `cargo` or `revdeprun` is not found immediately after installation,
restart your shell.

## Environment

Currently, this tool is designed for Ubuntu-based systems and requires:

- Operating system: Ubuntu 22.04 or newer.
- Version control: Git on `PATH`.
- Network access: To download R, R packages, and repository metadata.
- Elevated privileges: `sudo` access for installing R and system requirements.

Security note: Reverse dependency checks execute arbitrary third-party code.
Run `revdeprun` in temporary, isolated environments such as disposable cloud
instances or containers.

## Usage

Simply point `revdeprun` at your package:

```bash
revdeprun https://github.com/YOUR-USERNAME/YOUR-REPOSITORY.git
```

Git repository, local directory, or source tarball (`.tar.gz`) are supported.

Sensible defaults that make this fast and robust:

- Discover and install the current release version of R for Ubuntu.
- Pre-install system requirements for all reverse dependencies at once.
- Pre-install all dependencies required for checking reverse dependencies
  from the Posit Public Package Manager (P3M) binary repository,
  into a dedicated library in `revdep/library/`.
- Run `xfun::rev_check()` for parallel reverse dependency checking.
- Generate summary reports only for any check results with diffs.
- Use all available CPU cores for parallel installation and checking.

### Command-line options

```
Usage: revdeprun [OPTIONS] <REPOSITORY>

Arguments:
  <REPOSITORY>
          Git URL, local directory, or source package tarball (.tar.gz) for the target R package

Options:
      --r-version <R_VERSION>
          R version to install (e.g., release, 4.3.3, oldrel-1)
          [default: release]

      --num-workers <N>
          Number of parallel workers for xfun::rev_check()
          [default: number of CPU cores]

      --work-dir <WORK_DIR>
          Optional workspace directory where temporary files are created

      --skip-r-install
          Skip installing R and reuse the system-wide installation

  -h, --help
          Print help

  -V, --version
          Print version
```

## Example workflows

Standard check on a remote repository:

```bash
revdeprun https://github.com/YOUR-USERNAME/YOUR-REPOSITORY.git
```

Specify [R version](https://github.com/r-lib/actions/tree/v2-branch/setup-r#inputs)
and parallelism:

```bash
revdeprun --r-version devel --num-workers 48 \
  https://github.com/YOUR-USERNAME/YOUR-REPOSITORY.git
```

Use a custom workspace and SSH authentication:

```bash
revdeprun --work-dir /data/workspace \
  git@github.com:YOUR-USERNAME/YOUR-REPOSITORY.git
```

Check a local directory:

```bash
revdeprun ~/workspace/YOUR-REPOSITORY
```

Check a local source package tarball:

```bash
revdeprun ~/packages/YOURPACKAGE_1.2.3.tar.gz
```

Use an existing R installation:

```bash
revdeprun --skip-r-install https://github.com/YOUR-USERNAME/YOUR-REPOSITORY.git
```

## License

This project is licensed under the MIT License.
