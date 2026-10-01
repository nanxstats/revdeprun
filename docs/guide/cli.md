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
| `--skip-r-install` | Skip version resolution and installation; reuse system-wide R | Disabled |

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
revdeprun https://github.com/nanxstats/ggsci.git
```

To detach while checks continue, press **Ctrl+b**, release both keys, then
press **d**. You can now disconnect from SSH. After reconnecting to the same
instance as the same user, reattach to see progress:

```bash
tmux attach-session -t revdeprun
```

Keep the remote instance running until checks finish. A tmux session does not
survive a reboot or instance shutdown.

## Minimal, intentional repository edits

revdeprun tries hard not to modify the (local) source package. Two exceptions:

- It creates `revdep/` to hold the library for running reverse dependency checks.
- It appends `^revdep$` to `.Rbuildignore` (if needed) so building the package
  doesn't accidentally include the whole `revdep/` directory.
