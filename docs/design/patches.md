---
icon: lucide/drill
---

# Patches

When you run at CRAN scale, "small inefficiencies" become hours.

revdeprun includes two targeted monkey patches that remove bottlenecks in
upstream tooling without requiring forks.

## pkgdepends scheduler patch

pak embeds pkgdepends for install planning and scheduling. With thousands
of packages, two effects show up:

- The worker pool can under-fill when many small binary installs finish between
  polls.
- Dependency bookkeeping can become expensive for very large plans.

`assets/patch-pkgdepends.R` patches the scheduler to refill the worker pool more
aggressively and avoid unnecessary work. It is applied before `pak::pkg_install()`.

## xfun tarball download patch

`xfun::rev_check()` downloads reverse dependency tarballs from CRAN. Historically
this was serial, which is painful at 1,000+ packages.

`assets/patch-xfun.R` patches `xfun:::download_tarball()` to download in parallel
using `parallel::mcmapply()`. Concurrency is controlled by
`getOption("xfun.rev_check.download_cores")`.

## Why patch at runtime?

- It is the smallest change that can work.
- It keeps the Rust CLI focused on orchestration.
- It is easy to iterate, and easy to upstream when the patch proves out.

The cost is maintenance: upstream changes can break patches. That's why the
generated script "recipe" is covered by regression tests in `src/revdep.rs`.
