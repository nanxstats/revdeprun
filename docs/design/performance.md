---
icon: lucide/circle-gauge
---

# Performance

revdeprun is designed to optimize time-to-results. Its performance model is:

- Setup time is mostly network + I/O + some single-core work.
- Checking time is embarrassingly parallel and benefits strongly from more and faster CPU cores.

## The knobs

The CLI option `--num-workers` controls how many parallel workers
`xfun::rev_check()` uses. By default, revdeprun uses all available CPU cores.

Behind the scene, revdeprun computes a safe `--max-connections` value for
running `Rscript`. It also configures pak/pkgcache async HTTP connection
limits for binary downloads.

The auto-tuning heuristic for R's `--max-connections` option is implemented
in `src/util.rs` as:

```rust
max_connections = min(4096, ceil(max(128, 3 * Ncpus + 64) / 128) * 128)
```

## The bottlenecks

Here are the main historical bottlenecks and how revdeprun removes them:

- Binary install scheduler overhead: patched via `assets/patch-pkgdepends.R`.
- Serial revdep tarball downloads: patched via `assets/patch-xfun.R`.
- Slow "preparation" when `revdep/` is accidentally bundled: fixed by ensuring
  `.Rbuildignore` contains `^revdep$`.

## Scale tips

- On very large machines, raise file descriptor limits: `ulimit -n 10240`.
- Expect diminishing returns: the slowest individual packages dominate near the end.
- Disk matters: `R CMD check` can write a lot, so avoid tiny or slow storage.
