# Security policy

## Overview

**revdeprun executes untrusted third-party code with root privileges by design.**

This tool automates reverse dependency checking for R packages,
which requires executing arbitrary code from CRAN packages and
installing system dependencies with sudo access.
Never run it in any environment containing sensitive data.

## Security model

revdeprun requires:

1. **Disposable execution environment** - temporary instances destroyed after use
2. **No sensitive data** - no credentials, keys, or confidential information
3. **Root access** - sudo privileges for system package installation
4. **Network access** - downloads packages and metadata from external sources

All reverse dependencies are treated as potentially malicious.

## Critical security risks

### 1. Arbitrary code execution

Executes untrusted R code without sandboxing:

- Downloads and installs CRAN packages from reverse dependencies
- Runs R CMD check (examples, tests, vignettes)
- Compiles C/C++/Fortran code from source
- Full access to filesystem, network, and environment

### 2. Privileged system modifications

Requires extensive `sudo` usage:

- APT package installation and updates
- R installation from downloaded `.deb` files
- Executes arbitrary shell scripts with sudo from `pak::pkg_sysreqs()` output
- Creates system-wide symlinks in `/usr/local/bin/` and `/opt/`

### 3. Supply chain dependencies

Downloads from external services:

- R installers from `api.r-hub.io`
- CRAN/Bioconductor packages from Posit Public Package Manager
- Quarto releases from GitHub

### 4. Input processing risks

Processes untrusted inputs:

- Git clone from any URL without validation
- Tarball extraction without path traversal protection
- Potential git hooks execution

## Required: Use disposable environments

**Never run revdeprun on local machines, production systems, or anywhere with sensitive data.**

### Recommended environments

1. **Cloud VMs**: Cloud instances destroyed after use
2. **Containers**: Ephemeral Docker/Podman containers with no volume mounts
3. **CI/CD runners**: Fresh GitHub Actions/GitLab CI runners
   (not self-hosted on shared infrastructure)

## Best practices

1. Limit blast radius:
   - Use isolated cloud accounts/projects for checks
   - Never run on systems with access to other credentials

2. Always use the latest version from crates.io:

   ```bash
   cargo install revdeprun
   ```

3. Verify inputs before execution:
   - Confirm repository legitimacy
   - Review reverse dependency list for unexpected packages

4. Monitor during execution:
   - Watch for unusual resource usage or network activity

## Reporting vulnerabilities

### In revdeprun itself

**Do not open public issues.** Report via GitHub Security Advisories:

1. Go to https://github.com/nanxstats/revdeprun/security/advisories
2. Click "Report a vulnerability"
3. Include: description, reproduction steps, affected versions, impact

Response within 48 hours.

### Malicious packages discovered during checks

Report to:

- CRAN QA team: CRAN@R-project.org

## Version support

Only the latest stable release from crates.io receives security updates.

Check version: `revdeprun --version`

Update: `cargo install revdeprun`
