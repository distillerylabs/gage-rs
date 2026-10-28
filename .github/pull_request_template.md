## What and why

<!-- Link the issue this addresses (e.g. "Closes #123"). -->

## Checklist

- [ ] `make lint` and `make test` are clean locally
- [ ] Tests were added or updated (written before the implementation where practical)
- [ ] `crates/gage` has no direct printing, stdin/stdout/stderr use, or `std::process::exit`
- [ ] On-disk format change: `format_version` bump, new `tests/fixtures/format-v<N>/` fixture, and an ADR (see `doc/format/README.md`)
- [ ] Help snapshot changed (`tests/fixtures/help/`): intended, and the reason given
- [ ] No secrets, tokens, or real keys in code, tests, or docs
