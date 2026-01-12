---
icon: lucide/sliders-horizontal
---

# CLI reference

The CLI is intentionally small. Most behavior is encoded in the workflow.

## Synopsis

```text
revdeprun [OPTIONS] <REPOSITORY>
```

## Options

- `--r-version <R_VERSION>`: R version to install (`release` by default).
- `--num-workers <N>`: parallel workers for `xfun::rev_check()` (defaults to all cores).
- `--work-dir <WORK_DIR>`: use a specific workspace directory.
- `--skip-r-install`: reuse an existing system-wide R.

## Inputs

revdeprun accepts three types of package inputs in `<REPOSITORY>`:

### Git URL

```bash
revdeprun https://github.com/r-lib/usethis.git
revdeprun git@github.com:r-lib/usethis.git
```

The repository is cloned into the workspace clone root (by default, your
current directory). Clones use `--depth 1` for speed.

### Local directory

```bash
revdeprun ~/src/usethis
```

The directory is used as-is. revdeprun will create `revdep/` inside it.

### Source tarball

```bash
revdeprun ~/packages/usethis_3.0.0.tar.gz
```

The tarball is extracted into the workspace temp directory and used from there.

### Minimal, intentional repository edits

revdeprun tries hard not to modify your repository. There are two exceptions:

- It creates `revdep/` to hold the library.
- It appends `^revdep$` to `.Rbuildignore` (if needed) so building the package
  doesn't accidentally include the whole `revdep/` directory.
