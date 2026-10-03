# revdeprun

[![crates.io version](https://img.shields.io/crates/v/revdeprun)](https://crates.io/crates/revdeprun)
[![CI tests](https://github.com/nanxstats/revdeprun/actions/workflows/ci.yml/badge.svg)](https://github.com/nanxstats/revdeprun/actions/workflows/ci.yml)
[![Documentation](https://github.com/nanxstats/revdeprun/actions/workflows/docs.yml/badge.svg)](https://nanx.me/revdeprun/)

A command-line tool that automates reverse dependency checking for R packages.
Provision R on Ubuntu, install system dependencies, preinstall revdep
dependency binaries, configure environment context,
and run `xfun::rev_check()` in a single command.
Designed for cloud environments where you need reproducible, isolated test
runs without tedious manual setup.

## Installation

### Prerequisites

Install Rust and load its environment into the current shell:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

The `source` command adds Cargo's default binary directory, `$HOME/.cargo/bin`,
to `PATH` immediately in Bash or Zsh, so you can continue in the same terminal.

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

With the default Cargo installation, `revdeprun` is installed in the same
directory already added to `PATH`. Verify it is available in your current shell:

```bash
revdeprun --version
```

## Environment

Currently, this tool is designed for Ubuntu-based systems and requires:

- Operating system: Ubuntu 22.04, 24.04, 26.04, and future LTS releases.
- Version control: Git on `PATH`.
- Network access: To download R, R packages, and repository metadata.
- Elevated privileges: `sudo` access for installing R and system requirements.

## Security

> [!IMPORTANT]
> Never run `revdeprun` on your local machine or any environment with sensitive data.
> Reverse dependency checks execute arbitrary third-party R code, download
> dependencies from external repositories, and install system packages via sudo.
> Always run `revdeprun` in temporary, isolated environments such as disposable
> cloud instances or containers that will be destroyed after use.
>
> See [SECURITY.md](https://github.com/nanxstats/revdeprun/blob/main/SECURITY.md)
> for complete security guidelines.

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
       revdeprun <COMMAND>

Commands:
  bundle  Bundle check results into a .tar.zst archive for transfer
  help    Print this message or the help of the given subcommand(s)

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
          Skip R version resolution and installation; reuse the system-wide installation

  -h, --help
          Print help

  -V, --version
          Print version
```

## Example usages

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

This option bypasses the R Hub version API as well as R installation.
The system-wide `R` and `Rscript` commands must already be available on `PATH`;
`--r-version` is ignored when this option is set. Note that Quarto, pandoc,
and TinyTeX provisioning will also be skipped, so install any required
document toolchain in advance.

Debian is not an officially supported environment, but R installation is
available on a best-effort basis. If the R Hub API does not yet recognize the
detected Debian release, revdeprun probes successively older Debian releases
and uses the first compatible installer returned by the API.

## Monitor long-running checks

Reverse dependency checks can take hours to complete. Run them inside
[tmux](https://github.com/tmux/tmux/wiki/Getting-Started) on the remote Ubuntu
instance so they keep running if your SSH connection drops or your local
computer sleeps.

After connecting over SSH, install tmux and create a named session on the
remote instance:

```bash
sudo apt-get update && sudo apt-get install -y tmux
tmux new-session -s revdeprun
```

Inside that session, start your check:

```bash
revdeprun https://github.com/YOUR-USERNAME/YOUR-REPOSITORY.git
```

To detach while checks continue, press **Ctrl+b**, release both keys, then
press **d**. You can now disconnect from SSH. After reconnecting to the same
instance as the same user, reattach to see progress:

```bash
tmux attach-session -t revdeprun
```

Keep the remote instance running until checks finish. A tmux session does not
survive a reboot or instance shutdown.

## Transfer results

Cloud instances are often billed by the minute, so once a check finishes you
want the results off the machine quickly. `revdeprun bundle` packs everything
`xfun::rev_check()` left behind for review into a single zstd-compressed tar
archive:

- `00check_diffs.md` and `00check_diffs.html`: the summary of check results
  that differ between the CRAN and development versions of your package.
- `*.Rcheck/` and `*.Rcheck2/`: `R CMD check` output for each reverse
  dependency with problems.

The package sources, `revdep/library/`, and downloaded `tarball/` sources are
left out. Point the subcommand at the package directory printed at the end of
the check (for a Git URL, that is the clone in your current directory):

```bash
revdeprun bundle YOUR-REPOSITORY
```

The bundle is written next to the package directory as
`<package>-revdep.tar.zst`, and revdeprun prints an `scp` command to paste on
your local machine, followed by the `tar` command that extracts it:

```bash
scp ubuntu@HOST:/home/ubuntu/YOUR-REPOSITORY-revdep.tar.zst .
tar -xf YOUR-REPOSITORY-revdep.tar.zst
```

revdeprun fills in the user name, the absolute path, and the instance address
when the SSH session exposes a public one; otherwise replace `HOST` with the
address you connect to. Both GNU tar (1.31 or later) and macOS `tar` extract
`.tar.zst` archives through the `zstd` command, so install it if extraction
fails (`brew install zstd` on macOS, `sudo apt-get install zstd` on Ubuntu).
The archive extracts into a single directory named after the bundle file.

Use `--output` to choose a different file name or directory. The command refuses
to overwrite an existing file, and it exits with an error when the package
directory contains no results, which is the case when the check found no
problems to review.

```
Usage: revdeprun bundle [OPTIONS] [PACKAGE_DIR]

Arguments:
  [PACKAGE_DIR]
          Package directory where xfun::rev_check() ran (contains *.Rcheck/ and 00check_diffs.*)
          [default: .]

Options:
  -o, --output <FILE>
          Output archive path (.tar.zst); an existing directory receives the default file name

  -h, --help
          Print help
```

## Technical workflow

The following diagrams illustrate the `revdeprun` workflow.

### Phase 1: Environment setup

<img src="https://github.com/nanxstats/revdeprun/raw/main/assets/workflow-phase-1.svg">

### Phase 2: Dependency installation and reverse dependency checking

<img src="https://github.com/nanxstats/revdeprun/raw/main/assets/workflow-phase-2.svg">

## License

MIT
