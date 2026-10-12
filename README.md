# gage-rs

A Rust port of [`gage`](https://github.com/distillerylabs/gage) (`git` + `age`), a git-backed, age-encrypted secret and notes manager for the command line.

**Status: in progress, not usable yet.** Use [Go gage](https://github.com/distillerylabs/gage) for real secrets. The port aims for full on-disk compatibility: when it's done, a vault written by either binary can be read and written by the other.

- What gage is and how it works: the [Go gage README](https://github.com/distillerylabs/gage#readme) and the design docs in [doc/implementation/00_gage-cli/tdds/](doc/implementation/00_gage-cli/tdds/).
- How the port is being built: [the port plan](doc/implementation/00_gage-cli/plans/rust-port/index.md), and the [parity ledger](doc/implementation/00_gage-cli/plans/rust-port/parity-ledger.md) tracking what Go shipped against what has been ported.
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security: [SECURITY.md](SECURITY.md).

Licensed under [Apache-2.0](LICENSE).
