---
icon: lucide/cloud
---

# revdeprun

revdeprun is a Rust CLI that provisions R on Ubuntu and runs reverse dependency
checks for an R package in one command. It is usable by anyone and designed
for cloud instances: fast, reproducible, and disposable.

## Quick start

On a fresh Ubuntu LTS cloud instance:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
sudo apt-get update && sudo apt-get install -y build-essential
cargo install revdeprun
revdeprun https://github.com/YOUR-USERNAME/YOUR-REPOSITORY.git
```

## Why revdeprun?

Reverse dependency checks are ecosystem-scale integration tests: you check your
package by checking everything that depends on it.

At small scale, you can do this locally with tools like `{revdepcheck}` or
`xfun::rev_check()`. At large scale (hundreds or thousands of reverse
dependencies), the bottleneck is no longer "how do I run checks?", it is:

- How do I provision a clean cloud instance quickly and repeatedly?
- How do I install thousands of dependencies without compiling everything?
- How do I make the process reliable enough that I can trust the results?

revdeprun answers those questions by assuming a particular environment:
disposable Ubuntu LTS cloud instances, where you can pay for lots of CPU when
you need it, then delete the machine when you are done.

## What does revdeprun do?

In one command, revdeprun does the following:

- Installs (or reuses) a specific R version.
- Installs common tooling needed by `R CMD check` (Quarto, pandoc, TinyTeX).
- Resolves and installs Linux system requirements for the full reverse-dep set.
- Pre-installs R package dependencies, preferentially from binaries.
- Runs `xfun::rev_check()` for parallel reverse dependency checking.

## Why a Rust CLI?

This tool exists to make "fresh instance to finished revdep results" predictable.
Rust is a good fit because it gives you:

- A single executable with good error messages.
- A clear separation between orchestration (Rust) and the checking recipe (R).
- Tests that can run without having R installed.

## Design goals (and non-goals)

Design goals:

- One command that works on a clean cloud instance.
- Fast by default (binaries, parallelism, tuned networking).
- Deterministic and isolated.

Non-goals:

- "Safe to run anywhere." Revdep checks execute untrusted code by design.
- Supporting every Linux distribution. Ubuntu LTS is the target.
- Replacing {revdepcheck} or {xfun}. revdeprun is an orchestrator.
