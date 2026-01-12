---
icon: lucide/flag
---

# Results

All run artifacts live under the package directory.

## Key paths

- `revdep/library/`: isolated R library containing pre-installed dependencies.
- `00check_diffs.md`: summary of packages whose check results changed.
- `00check_diffs.html`: the same summary rendered as HTML.
- `*.Rcheck/` and `*.Rcheck2/`: per-package check outputs.

The exact directory layout is controlled by `xfun::rev_check()`.
No check summary files and per-package directories are created if
zero reverse dependency check issues were found.

## How to interpret diffs

`xfun::rev_check()` checks each reverse dependency twice:

- Against the CRAN version of your package.
- Against the development version of your package.

The diff report helps you focus on regressions that are likely caused by your
changes, not by unrelated breakage on CRAN. False positives can still occur,
so use your judgment when interpreting the results. False negatives are much
less likely.
