---
icon: lucide/circle-gauge
---

# Performance

revdeprun is designed to optimize time-to-results. Its performance model is:

- Setup time is mostly network + I/O + some single-core work.
- Checking time is embarrassingly parallel and benefits strongly from more and faster CPU cores.

## Scaling tips

- On very large machines, raise file descriptor limits: `ulimit -n 10240`.
- Expect diminishing returns: the slowest individual packages dominate near the end.
- Disk matters: `R CMD check` can write a lot, so avoid tiny or slow storage.

## Parallel workers for reverse dependency check

The CLI option `--num-workers` controls how many parallel workers
`xfun::rev_check()` uses. By default, revdeprun uses all available CPU cores.

## Binary package downloads

revdeprun configures pak/pkgcache async HTTP connection limits to accelerate
binary package downloads from P3M.

By default, binary package downloads use the same computed max connections
with a 50 connections per host cap, increasing from the pkgcache default of
6 connections per host.

## Source tarball downloads

`xfun::rev_check()` downloads reverse dependency tarballs from CRAN.
It supports parallel downloads for large reverse dependency sets
since 0.56 ([yihui/xfun#112](https://github.com/yihui/xfun/pull/112)).

Concurrency is controlled by `getOption("xfun.rev_check.download_cores")`.
revdeprun sets it to 50, instead of the xfun default of the number of CPU cores.

## Auto-tuning `--max-connections`

Behind the scene, revdeprun computes a safe `--max-connections` value for
running `Rscript`. The `--max-connections` argument is available since R 4.4.0.

When `xfun::rev_check()` runs with many parallel workers, the main R process
can run out of connections (pipes for inter-process communication + various
other connections opened during installs and checks). To avoid that, revdeprun
auto-tunes `Rscript --max-connections` based on the worker count
(`--num-workers`).

The heuristic is implemented in `src/util.rs` (`util::optimal_max_connections`)
as:

```rust
max_connections = min(4096, ceil(max(128, 3 * Ncpus + 64) / 128) * 128)
```

`Ncpus` is the number of parallel workers revdeprun will use.

### Why this works

- The main process typically needs about 1 to 2 R connections per worker
  (pipes for result/control) plus a small fixed overhead.
- Multiplying workers by 3 bakes in ~50% headroom over a 2-per-worker baseline
  for miscellaneous connections opened by your code and the parallel machinery.
- The extra `+64` is a fixed cushion for the own connections of the main process.
- It respects R's hard cap (`4096`) and the legacy default (`128`), and rounds
  up to the next multiple of `128` for stable "round number" values.

### Example values

The unrounded baseline is `max(128, 3 * Ncpus + 64)`. revdeprun then rounds up
to the next multiple of `128` and caps the final value at `4096`.

- 16 cores &rarr; computed `112` &rarr; `128` (lift to floor)
- 32 cores &rarr; computed `160` &rarr; `256`
- 128 cores &rarr; computed `448` &rarr; `512`
- 256 cores &rarr; computed `832` &rarr; `896`
- 384 cores &rarr; computed `1216` &rarr; `1280`

## pkgdepends scheduler patch

pak embeds pkgdepends for install planning and scheduling. With thousands
of packages, two effects show up:

- The worker pool can under-fill when many small binary installs finish
  between polls.
- Dependency bookkeeping can become expensive for very large plans.

`assets/patch-pkgdepends.R` patches the scheduler to refill the worker pool
more aggressively and avoid unnecessary work.
The monkey patch is applied before calling `pak::pkg_install()`.
