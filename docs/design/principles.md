---
icon: lucide/lightbulb
---

# Principles

revdeprun is a toolchain wrapper. The "secret sauce" is not any single trick,
but a set of decisions that make reverse dependency checks fast and predictable
in a cloud setting.

## Design principles

- Assume cloud: disposable Ubuntu instances, internet access, no secrets.
- Sensible defaults: work out of the box without tuning.
- Prefer binaries: compile as little as possible.
- Make parallelism the default: checks scale well; installs mostly do.
- Treat the network as unreliable: retry, deduplicate, and keep going.
- Keep the interface lean: expose the meaningful knobs, not every knob.

## The main insight: separate setup from checking

Most "revdep pain" comes from setup: provisioning R, system libraries,
and thousands of R dependencies. Once that is done, the actual checking
is the easy part: `xfun::rev_check()` is excellent at using all your CPU cores.

That separation shows up everywhere in the implementation and is the reason the
tool is structured as a two-phase [workflow](workflow.md).
