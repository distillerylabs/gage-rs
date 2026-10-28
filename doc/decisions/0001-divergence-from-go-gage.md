# ADR-0001: gage-rs diverges from Go gage and becomes its own project

- Status: Accepted
- Origin: the [divergence plan](../implementation/00_gage-cli/plans/divergence/index.md#resolved-decisions); archived `Q-COMPAT` ([entry](../archive/rust-port/open-questions.md#q-compat))

## Context

gage-rs began as a port of Go gage (`git` + `age`, written in Go). The
port reached behavioral parity with Go gage at commit `64b6cf0` and
followed Go as its spec: Go's behavior, test inventory and help output
were the reference, and a ledger plus an inventory guard enforced it.

Go gage is being deprecated, and development continues only here. Keeping
the repo built to follow a reference that no longer moves means every later
change is judged against it, and its patterns keep spreading.

## Decision

gage-rs is its own project and its own reference implementation.

- **On-disk format.** v2 is frozen as gage-rs's own format. The spec is
  [doc/format/v2.md](../format/v2.md). It changes only through a
  `format_version` bump, a new spec, a new fixture and a migration ADR (see
  [doc/format/README.md](../format/README.md)). Nothing is promised about
  vaults Go gage wrote, in either direction.
- **Port-era documents.** The port plans, the `Q-` register and the parity
  ledger are archived and frozen in [doc/archive/](../archive/README.md).
  The design documents in [doc/design/](../design/) are gage-rs's own. This
  log replaces the register: every decision that still constrains code is
  an ADR here.
- **CLI behavior.** User-visible behavior stays as it was at the end of the
  port, and is now gage-rs's contract rather than Go's:
  - help layout and flags, including `-v/--version` and the hidden
    `--script/--stdin/--yes`
  - exit codes 0–7 ([ADR-0037](0037-exit-code-taxonomy.md))
  - the duration grammar and spelling
  It is pinned by the snapshots under `tests/fixtures/help/`.
- **Retired.** The parity ledger, the Go test inventory and its guard, the
  Go-pinned help captures and the differential-comparison tooling.
- **Release / 1.0.** A separate effort, later.

## Consequences

- When a gage-rs behavior and Go gage's disagree, nothing is wrong with
  gage-rs.
- A change to the format or to the pinned CLI surface is a decision, and
  gets an ADR.
- Code comments that cite the archived register are rewritten by the source
  sweep (D2); each ADR below lists its origin anchors so that mapping is
  mechanical.
