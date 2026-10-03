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

## Transfer results

Cloud instances are often billed by the minute, so once a check finishes you
want the results off the machine quickly. `revdeprun bundle` packs the review
artifacts above into a single zstd-compressed tar archive, leaving out the
package sources, `revdep/library/`, and the downloaded `tarball/` sources:

```bash
revdeprun bundle ggsci
```

The bundle is written next to the package directory as
`<package>-revdep.tar.zst`. When it is done, revdeprun prints an `scp` command
to paste on your local machine and the `tar` command that extracts it:

```bash
scp ubuntu@HOST:/home/ubuntu/ggsci-revdep.tar.zst .
tar -xf ggsci-revdep.tar.zst
```

revdeprun fills in the user name, the absolute path, and the instance address
when the SSH session exposes a public one; otherwise replace `HOST` with the
address you connect to. The archive extracts into a single directory named after
the bundle file, so the layout above is preserved under `ggsci-revdep/`.

Both GNU tar (1.31 or later) and macOS `tar` extract `.tar.zst` archives through
the `zstd` command. Install it if extraction fails:

```bash
brew install zstd              # macOS
sudo apt-get install -y zstd   # Ubuntu
```

If the check found no problems, there are no `*.Rcheck/` directories or summary
files, and `revdeprun bundle` exits with an error saying so. The end-of-check
summary tells you which case you are in.
