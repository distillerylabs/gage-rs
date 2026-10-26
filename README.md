# gage-rs

A Rust port of [`gage`](https://github.com/distillerylabs/gage) (`git` + `age`), a git-backed, age-encrypted secret and notes manager for the command line.

**Status: behavioral parity reached, not released.** gage-rs reaches behavioral parity with Go gage at commit [`64b6cf0`](https://github.com/distillerylabs/gage/commit/64b6cf078f3e756efb9d868cc1518dc1d9060748): the same commands, behavior and vault layout. It has not been released or independently audited, so **use [Go gage](https://github.com/distillerylabs/gage) for real secrets** until a release decision is made. It does not read vaults Go gage wrote, and Go's exact bytes and help layout are not reproduced.

- Using gage-rs: [getting started](doc/cli/getting-started.md), then the rest of [doc/cli/](doc/cli/README.md). Build it with `make build`.
- What gage is and how it works: the [Go gage README](https://github.com/distillerylabs/gage#readme) and the design docs in [doc/implementation/00_gage-cli/tdds/](doc/implementation/00_gage-cli/tdds/).
- How the port was built: [the port plan](doc/implementation/00_gage-cli/plans/rust-port/index.md), and the [parity ledger](doc/implementation/00_gage-cli/plans/rust-port/parity-ledger.md) mapping what Go shipped to where it landed.
- Where gage-rs deliberately differs from Go gage: the [open-questions register](doc/implementation/00_gage-cli/plans/rust-port/open-questions.md), one `Q-` entry per decision.
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security: [SECURITY.md](SECURITY.md).

Licensed under [Apache-2.0](LICENSE).
