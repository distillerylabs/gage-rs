# Go reference fixtures

Captured from Go gage at the commit in `GO_GAGE_SHA`, and regenerated only by
`scripts/go-reference/capture.sh <go-gage-checkout>`. Don't edit by hand.

- `help/`: `gage --help` (`root.txt`) and `gage help <cmd>` for every
  one-shot command (`vault_list.txt` = `gage help vault list`). gage-rs's help
  output is compared against these; see the parity ledger §4. Session-mode
  help is not captured yet (Q-SESSION-HELP-FIXTURES).
- `test-inventory.tsv`: every Go test function as `phase  file  test`, tagged
  by `scripts/go-reference/phase-map.awk`. Milestones claim rows from it; see
  the parity ledger §5.

Go-written vaults, identity files and entries for cross-implementation tests
will live under `vaults/`, added by the milestones that touch on-disk bytes.
