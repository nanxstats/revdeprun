---
icon: lucide/download
---

# Install

revdeprun is built for Ubuntu LTS cloud instances. You will need:

- Ubuntu 22.04, 24.04, 26.04 (or newer LTS).
- `git` on `PATH`.
- `sudo` access (for R + system requirements).
- Internet access (CRAN metadata, R installers, packages).

## Install Rust

Install Rust and load its environment into the current shell:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

The `source` command adds Cargo's default binary directory, `$HOME/.cargo/bin`,
to `PATH` immediately in Bash or Zsh. You can continue in the same terminal.
In Bash, `source ~/.bashrc` also works if that file loads Cargo's environment.

## Install build tools

```bash
sudo apt-get update && sudo apt-get install -y build-essential
```

## Install revdeprun

From crates.io:

```bash
cargo install revdeprun
```

From GitHub:

```bash
cargo install --git https://github.com/nanxstats/revdeprun.git
```

With the default Cargo installation, `revdeprun` is installed in the same
directory already added to `PATH`. Verify it is available in your current shell:

```bash
revdeprun --version
```

For checks over SSH, [start a tmux session](cli.md#monitor-long-running-checks)
on the remote instance before running revdeprun.

## If you are using a huge machine

On very high core-count instances you can hit the file descriptor limit during
parallel downloads/installs. Raising it is often enough:

```bash
ulimit -n 10240
```
