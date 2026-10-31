# gage-core

The library behind [gage](https://github.com/distillerylabs/gage-rs), a
git-backed, age-encrypted secret and notes manager: vaults, entries,
sessions, recipients and sync. It does no terminal I/O; anything that needs a
human decision comes back as a typed value for the frontend to render.

The crate is `gage-core`; the library is imported as `gage`:

```rust
use gage::{Vault, Session, Entry};
```

The `test-support` feature exposes test fixtures and is outside semver.

- Command-line tool: [`gage-cli`](https://crates.io/crates/gage-cli)
- On-disk format, design and decisions: <https://github.com/distillerylabs/gage-rs>

Licensed under Apache-2.0.
