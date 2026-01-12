---
icon: lucide/layers
---

# System requirements

On Linux, R packages often depend on system libraries: `libcurl`, `libxml2`,
`libssl`, and so on. If you only install system requirements package-by-package
you pay a huge overhead, and you get a lot of nondeterministic failures.

revdeprun takes the opposite approach: resolve system requirements for the
entire reverse dependency set up front, then install them in bulk.

## How it works

1. In the package directory, an R script computes the reverse dependencies using
   `available.packages()` + `tools::package_dependencies(reverse = TRUE)`.
2. It calls `pak::pkg_sysreqs(revdeps, sysreqs_platform = "ubuntu")`.
3. The script prints a JSON payload (install scripts + post-install hooks).
4. Rust parses the JSON and executes each command with `sudo`.

See `src/sysreqs.rs`.

## Why this matters

- It is faster: you avoid a "thousand tiny apt installs".
- It is more reliable: one coherent pass over system deps reduces flakiness.
- It keeps the checking phase focused on R, not OS plumbing.
