---
icon: lucide/download
---

# Install

revdeprun is built for Ubuntu LTS cloud instances. You will need:

- Ubuntu 22.04 or 24.04 (or newer LTS).
- `git` on `PATH`.
- `sudo` access (for R + system requirements).
- Internet access (CRAN metadata, R installers, packages).

## Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Restart your shell (so `cargo` is on `PATH`).

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

## If you are using a huge machine

On very high core-count instances you can hit the file descriptor limit during
parallel downloads/installs. Raising it is often enough:

```bash
ulimit -n 10240
```
