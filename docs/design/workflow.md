---
icon: lucide/workflow
---

# Workflow

The workflow is almost linear: do all provisioning up front, then run the
checks with minimal surprises.

## Phase 1: provision and prepare

```mermaid
flowchart LR
    Start([Input]) --> Setup[Prepare Workspace<br/>Resolve R Version]
    Setup --> CheckR{--skip-r-install?}

    CheckR -->|No| InstallR[Install R<br/>if not detected]
    CheckR -->|Yes| InstallDocs
    InstallR --> InstallDocs[Install Quarto,<br/>pandoc, TinyTeX<br/>if not detected]
    InstallDocs --> PrepRepo{Repository<br/>Type}

    PrepRepo -->|Git URL| Clone[git clone]
    PrepRepo -->|Local Dir| UseLocal[Use as-is]
    PrepRepo -->|.tar.gz| Extract[Extract]

    Clone --> Next([To Phase 2])
    UseLocal --> Next
    Extract --> Next

    style Start fill:#f4cccc
    style Next fill:#f4cccc
    style Setup fill:#fce5cd
    style CheckR fill:#fce5cd
    style InstallR fill:#d9ead3
    style InstallDocs fill:#d9ead3
    style PrepRepo fill:#fce5cd
    style Clone fill:#fff2cc
    style UseLocal fill:#fff2cc
    style Extract fill:#fff2cc
```

This is implemented in `src/lib.rs` by orchestrating:

- `src/workspace.rs`: workspace directories
- `src/r_version.rs`: resolve R version spec to a concrete installer
- `src/r_install.rs`: install R + tooling (or reuse existing)
- `src/revdep.rs`: prepare repository input

## Phase 2: install dependencies and check

```mermaid
flowchart LR
    Start([From Phase 1]) --> ResolveSys[Resolve System<br/>Requirements]
    ResolveSys --> InstallSys[Install apt<br/>Packages]
    InstallSys --> PreInstall[Pre-install Binaries<br/>into revdep/library/]
    PreInstall --> RunCheck[xfun::rev_check#40;#41;<br/>Parallel Checks]

    RunCheck --> Output1[revdep/library/]
    RunCheck --> Output2[00check_diffs.md<br/>00check_diffs.html]
    RunCheck --> Output3[\*.Rcheck/<br/>\*.Rcheck2//]

    Output1 --> End([Complete])
    Output2 --> End
    Output3 --> End

    style Start fill:#f4cccc
    style End fill:#f4cccc
    style ResolveSys fill:#fce5cd
    style InstallSys fill:#d9ead3
    style PreInstall fill:#fce5cd
    style RunCheck fill:#c9daf8
    style Output1 fill:#fff2cc
    style Output2 fill:#fff2cc
    style Output3 fill:#fff2cc
```

The two key pieces are:

- `src/sysreqs.rs`: resolve + install Linux system requirements for *all* revdeps.
- `src/revdep.rs`: generate deterministic R scripts to install dependencies and
  run `xfun::rev_check()`.

## Workspace layout

By default:

- Remote repos clone next to your current directory.
- Temporary files go in `./revdeprun-work/`.

With `--work-dir`, both clones and temporary files go under that directory.
