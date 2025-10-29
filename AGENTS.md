# Guidance for Codex

This project ships a Rust CLI that provisions R on Ubuntu and automates reverse
dependency checks for R packages.

## Architectural outline

- `src/lib.rs` exposes `run()`, which wires together argument parsing, workspace
  creation, R toolchain resolution and installation, repository preparation, and
  the final `revdepcheck` invocation.
- `src/cli.rs` uses `clap` for argument parsing. Keep the CLI surface lean; new
  flags require corresponding documentation updates.
- `src/r_version.rs` talks to `https://api.r-hub.io/rversions/resolve`. Changes
  here must continue to support setup-r style shorthand (e.g. `release`,
  `oldrel-1`). Prefer blocking `reqwest` to avoid pulling tokio into the call
- `src/r_install.rs` downloads the `.deb`, installs prerequisites, and creates
  `/usr/local/bin` symlinks with `xshell`. Assume Ubuntu-only environments.
- `src/revdep.rs` clones repositories, writes a bootstrap R script, and runs
  `revdepcheck::revdep_check`. Keep the script deterministic and avoid editing
  user repositories outside `revdep/`. The revdep flow uses two scripts:
  one quiet bootstrap script for installing dependencies and a second script
  that runs `revdepcheck`, so only the interactive phase reaches stdout.
- `src/workspace.rs` manages per-run directories. Respect user-provided
  workspaces without deleting their content.
- `src/util.rs` holds shared helpers; keep it small and well-tested.

## Operational expectations

- Tests (`cargo test`) and doc tests should run on macOS and Linux without
  needing R. Avoid adding integration tests that require R installation.
- Reuse `xshell` for shell calls instead of `std::process::Command` directly.
- Keep new dependencies minimal and compatible with the MSRV declared in
  `Cargo.toml`.
- If the revdep recipe changes, reflect it in `build_revdep_script` and add a
  regression test that checks for critical fragments.
- README changes must mirror CLI options and behavioral adjustments.
- Update `CHANGELOG.md` using Keep a Changelog conventions.
