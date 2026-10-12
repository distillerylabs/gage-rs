## What and why

<!-- Link the issue this addresses (e.g. "Closes #123"). -->

## Checklist

- [ ] `make lint` and `make test` are clean locally
- [ ] Tests were added or updated (written before the implementation where practical)
- [ ] `crates/gage` has no direct printing, stdin/stdout/stderr use, or `std::process::exit`
- [ ] Parity: the milestone's ledger rows and Go tests are accounted for (ported, or a written reason), and help output matches `tests/fixtures/go/help/` or the deviation is recorded
- [ ] On-disk format is unchanged relative to Go gage, or the change is recorded in `plans/rust-port/open-questions.md`
- [ ] No secrets, tokens, or real keys in code, tests, or docs
