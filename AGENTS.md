# AGENTS.md

This project ships a Rust CLI that provisions R on Ubuntu and automates reverse
dependency checks for R packages.

## Architectural outline

- `src/lib.rs` exposes `run()`, which wires together argument parsing, workspace
  creation, R toolchain resolution and installation, repository preparation, and
  the final `xfun::rev_check()` invocation.
- `src/cli.rs` uses `clap` for argument parsing. Keep the CLI surface lean; new
  flags require corresponding documentation updates.
- `src/r_version.rs` talks to `https://api.r-hub.io/rversions/resolve`. Changes
  here must continue to support setup-r style shorthand (e.g. `release`,
  `oldrel-1`). Prefer blocking `reqwest` to avoid pulling tokio into the call
- `src/r_install.rs` downloads the `.deb`, installs prerequisites, and creates
  `/usr/local/bin` symlinks with `xshell`. Assume Ubuntu-only environments.
- `src/revdep.rs` clones repositories, assembles the dependency install and
  `xfun::rev_check()` scripts from the R sources under `assets/r/`, and runs
  them after preinstalling binaries from Posit Package Manager. Keep both
  scripts deterministic and avoid editing user repositories outside `revdep/`.
  Only the interactive `xfun::rev_check()` phase should reach stdout.
- `src/sysreqs.rs` resolves Ubuntu system requirements for all reverse
  dependencies with `assets/r/sysreqs.R` and installs them in bulk.
- `src/r_scripts.rs` embeds every `.R` file under `assets/r/` at compile time
  with `include_str!` and assembles runnable scripts by prepending a
  Rust-generated configuration block (`RConfig`) of plain R assignments.
  R logic belongs in the `.R` files; configuration values and process
  orchestration stay in Rust. Do not introduce a templating engine or
  placeholders in the R sources. Each `.R` file documents the configuration
  variables it expects in its header comment.
- `src/workspace.rs` manages workspace directories. Remote clones default to
  `<repo>` alongside the current working directory, while temporary files live
  under `revdeprun-work/`. Respect user-provided workspaces without deleting
  their content.
- `src/util.rs` holds shared helpers; keep it small and well-tested.

## Operational expectations

- Tests (`cargo test`) and doc tests should run on macOS and Linux without
  needing R. Avoid adding integration tests that require R installation.
- Reuse `xshell` for shell calls instead of `std::process::Command` directly.
- Keep new dependencies minimal and compatible with the MSRV declared in
  `Cargo.toml`.
- If the revdep recipe changes, edit the relevant `.R` file under `assets/r/`
  (and `script_config` in `src/revdep.rs` if a new configuration value is
  needed), then add a regression test in `src/revdep.rs` that checks the
  assembled script for critical fragments and ordering.
- README changes must mirror CLI options and behavioral adjustments.
- Update `CHANGELOG.md` using Keep a Changelog conventions.
