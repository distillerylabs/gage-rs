# Go reference fixtures

Captured from Go gage at the commit in `GO_GAGE_SHA`, and regenerated only by
`scripts/go-reference/capture.sh <go-gage-checkout>`. Don't edit by hand.

- `help/`: `gage --help` (`root.txt`) and `gage help <cmd>` for every
  one-shot command (`vault_list.txt` = `gage help vault list`). A reference
  for each command's name, aliases, group, arguments and flags, not a byte
  gate: gage-rs renders help with clap (Q-HELP-RENDER, parity ledger §4).
- `test-inventory.tsv`: every Go test function as `phase  file  test`, tagged
  by `scripts/go-reference/phase-map.awk`. Milestones claim rows from it; see
  the parity ledger §5.

No Go-written vaults are kept here: gage-rs targets behavioral parity, not
cross-binary compatibility (Q-COMPAT).
