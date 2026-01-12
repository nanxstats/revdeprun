---
icon: lucide/terminal
---

# CLI reference

The CLI is intentionally clean and minimal. Most behavior is encoded
in the workflow with sensible defaults.

## Synopsis

```text
revdeprun [OPTIONS] <REPOSITORY>
```

## Options

| Name | Description | Default |
|------|-------------|---------|
| `--r-version <R_VERSION>` | R version to install | `release` |
| `--num-workers <N>` | Parallel workers for `xfun::rev_check()` | All CPU cores |
| `--work-dir <WORK_DIR>` | Use a specific workspace directory | Current directory |
| `--skip-r-install` | Reuse an existing system-wide R | Disabled |

## Inputs

revdeprun accepts three types of package inputs.

### Git URL

```bash
revdeprun https://github.com/nanxstats/ggsci.git
```

The repository is cloned into the workspace clone root
(by default, your current directory). Clones use `--depth 1` for speed.

### Local directory

```bash
revdeprun ~/packages/ggsci
```

The directory is used as-is. revdeprun will create `revdep/` inside it.

### Source tarball

```bash
revdeprun ~/packages/ggsci_4.0.0.tar.gz
```

The tarball is extracted into the workspace temp directory and used from there.

## Minimal, intentional repository edits

revdeprun tries hard not to modify the (local) source package. Two exceptions:

- It creates `revdep/` to hold the library for running reverse dependency checks.
- It appends `^revdep$` to `.Rbuildignore` (if needed) so building the package
  doesn't accidentally include the whole `revdep/` directory.
