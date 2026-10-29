# D3c — Small Go-isms

[D3 plan](index.md) · [#134](https://github.com/distillerylabs/gage-rs/issues/134) · previous: [D3b](d3b-error-boundary.md)

> **Recommended model: Sonnet.** Four local refactors, each with its tests
> already in place. The one place to slow down is `HeldVault`: it holds
> an `Identity`, and an identity must still be dropped, zeroized and
> unlocked on every path that leaves the unlocked state.

## Goal

Four Go shapes go:

- `Identity::new` returns a value instead of a `(T, Option<E>)` tuple.
- The session's per-vault state is an enum in which "locked but indexed"
  can't be represented.
- `marshal`/`unmarshal` get Rust names.
- The config-like modules share one load/save vocabulary.

Nothing user-visible changes. The help snapshots and `format_v2_*` tests
are unedited.

## Depends on

D3b. D3b settles the error types that the renamed load and save
functions return.

## Design references

- [D3 index](index.md).
- [ADR-0023](../../../../../decisions/0023-identity-secret.md): how the
  identity secret is held. Page-locking, and zeroizing on `Drop`.
- [ADR-0029](../../../../../decisions/0029-index-arena.md): the index
  arena. It locks pages only when the key's lock succeeded.
- CLAUDE.md: "`Identity` zeroizes and releases its page lock in `Drop`;
  there is no separate `close()` to forget", and "The in-session
  metadata index … lives on `Session`".

## Where things are

**`Identity::new`.**

- Defined at `identity.rs:103-119`, as `pub(crate) fn new(device, key,
  locker) -> (Identity, Option<io::Error>)`.
- Called from `vault.rs:620` (`Vault::unlock`) and `recovery/swap.rs:329`.
  Both call `warn_page_lock_failed` (`vault.rs:820`) when the lock
  failed.
- `Identity::page_locked()` feeds `Index::new` (`session.rs:710`).
- It is the only `(T, Option<E>)` return in the library.

**`HeldVault`** (`session.rs:179-194`) has these fields:

- `vault`
- `identity: Option<Identity>`
- `last_used: Option<Instant>`
- `index: Option<Index>`
- `index_warned: bool`
- `pulled: Arc<AtomicBool>`

Its states are implicit:

| State | `identity` | `index` |
|---|---|---|
| Locked | `None` | `None` |
| Unlocked, no index | `Some` | `None` |
| Unlocked and indexed | `Some` | `Some` |

`lock_held` keeps the invalid `None`/`Some` combination out only by
calling `discard_index` first. Twelve `expect`s assume a state (lines
289-518).

**`entry::marshal`/`unmarshal`** (`entry/yaml.rs:45,360`) are
re-exported from `entry/mod.rs:31`. They are called from `entry/store.rs`
and, in `gage-cli`, from `editor.rs` and `commands/entry.rs`.

**Read and write names** in the config-like modules:

| Module | Read | Write |
|---|---|---|
| `config` | `read` (missing file is an error), `load` (missing file gives the default) | `write`, `save` (makes the directory) |
| `vaultconfig` | `read` | `write` |
| `recipients` | `read` | `write` |
| `identityfile` | `read` (missing file gives `None`), `open` (decrypts) | `create` |
| `remoteauth` | `load` | `store` |
| `trustcache` | `Vault::load_trust_cache` | `Vault::store_trust_cache` |

## Decisions to make first

1. **The page-lock return.** Proposed: `Identity::new(...) -> Unlocked`,
   where `struct Unlocked { identity: Identity, page_lock: PageLock }`
   and `enum PageLock { Locked, Failed(io::Error) }`. `Identity`'s
   `page_locked: bool` becomes `PageLock`'s discriminant.

   Alternative: `Identity::new -> Identity`, with `Identity::page_lock()
   -> &PageLock` stored on it. This is simpler, but then the
   `io::Error` lives as long as the key does. Pick one and record it.
2. **The `HeldVault` shape.** Proposed:

   ```rust
   struct HeldVault { vault: Vault, pulled: Arc<AtomicBool>, state: Held }
   enum Held {
       Locked,
       Unlocked { identity: Identity, last_used: Instant, index: Option<Indexed> },
   }
   struct Indexed { index: Index, warned: bool }
   ```

   - `last_used` is an `Instant` while unlocked. Today it is `None` only
     before first use; confirm that from the code first.
   - `lock_held` becomes `state = Held::Locked`. Drop order puts the
     index before the identity, so check that the index arena is
     released before the key. Write it out explicitly if field order
     isn't enough.
   - `refresh_entry`'s redundant identity match goes.
3. **The entry YAML names.** Proposed: `Entry::to_yaml(&self) ->
   Result<Zeroizing<Vec<u8>>, Error>` and `Entry::from_yaml(&[u8]) ->
   Result<Entry, Error>`. The free functions go.
4. **One load/save vocabulary.** Proposed:
   - `load` reads and parses. A missing file is `Ok(None)` where absence
     is normal (the trust cache, an identity file, a token) and an error
     where it isn't (the vault config, `.age-recipients`).
   - `save` writes atomically.
   - `config::load_or_default` and `config::save` keep today's
     semantics, under names that say them.
   - `identityfile::open`, which decrypts, stays `open`. Decrypting is
     not loading.
   - `create` stays where it refuses to overwrite.
   - `remoteauth::store` becomes `save`.

   Record the final table here before renaming anything.

## Tasks

### 1. `Identity::new`

- [ ] Tests first. In `unlock_tests.rs`/`swap_tests.rs`, the page-lock
      failure cases assert on `PageLock::Failed` instead of a tuple
      element. `page_lock_failure_warns_once_and_proceeds` stays green
      unedited.
- [ ] Implement decision 1. Callers: `vault.rs:620`, `swap.rs:329`, and
      `Index::new`'s `lock_enabled`.

### 2. `HeldVault`

- [ ] Tests first:
  - lock, then unlock, then index, then lock again: the identity is
    dropped and the index is gone
  - a pull while indexed drops the index and keeps the identity
  - the idle timeout locks

  Most of these exist in `session_tests.rs`; add only the missing ones.
- [ ] Implement decision 2. The `expect`s become matches, or move into
      accessor methods that return the state they need.
- [ ] `status()` and `enter()` read the enum.

### 3. Entry YAML names

- [ ] Rename per decision 3, in `entry/store.rs`, `editor.rs`,
      `commands/entry.rs` and the tests. This is a rename only; no test
      body changes beyond the call.

### 4. Load and save

- [ ] Settle decision 4's table. Rename across the library and the CLI
      callers (`repl.rs`, `shell.rs`, `complete.rs`, `commands/*`).
- [ ] Where a rename changes missing-file semantics, which it shouldn't,
      add a test first.

### 5. Green

- [ ] `cargo check --workspace --all-targets` after each task.
- [ ] `git diff --stat main -- tests/fixtures` is empty.

## Definition of done

- Every box above is ticked, and decisions 1–4 are recorded here as
  settled.
- No `(T, Option<E>)` return remains in `crates/gage`.
- `HeldVault` has no `Option<Identity>`.
- `make lint` and `make test` are clean. The help snapshots and
  `format_v2_*` tests are green and unedited.
- CI is green on Linux, macOS and Windows.
