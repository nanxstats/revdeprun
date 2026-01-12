---
icon: lucide/shield-check
---

# Security

Reverse dependency checks are *not* a safe workload.

They execute arbitrary R code from third-party packages, download content from
multiple external services, and install system packages with `sudo`.

If you take only one thing away from this documentation, make it this:

!!! danger "Do not run revdeprun on a machine with secrets"
    Use a disposable cloud instance or container. Assume compromise. Destroy
    the environment when the run finishes.

## Threat model

You should assume a reverse dependency can:

- Read any file your user can read.
- Exfiltrate anything it can reach over the network.
- Abuse `sudo` if it can influence how you provision the environment.

revdeprun reduces friction. It does not make the workload trustworthy.

## Safe operating practices

- Use a dedicated, short-lived cloud VM with no long-lived credentials.
- Prefer instance profiles/roles with minimal permissions (or none at all).
- Avoid mounting shared volumes.
- Do not run on your laptop, workstation, or CI runners with production access.
- Treat the output as untrusted too (for example, HTML reports can contain surprises).

For complete security guidelines, see
[SECURITY.md](https://github.com/nanxstats/revdeprun/blob/main/SECURITY.md).
