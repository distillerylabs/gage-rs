# Help snapshots

gage-rs's own help output, one file per surface, compared by
`help_snapshots_match` in `crates/gage-cli/src/help_snapshot_tests.rs`.

- `root.txt`: `gage help`.
- `<command>.txt`: `gage help <command path>`, with spaces as `_`
  (`vault_list.txt`), for every registered command and parent.
- `session.txt`: the in-session `help` listing.
- `usage-error_<parent>.txt`: the error for an unknown subcommand under a
  parent.

A help change shows up as a diff here. To accept an intended one:

```
UPDATE_SNAPSHOTS=1 cargo test -p gage-cli --lib help_snapshots_match
```

That run rewrites the files and fails on purpose. Review
`git diff tests/fixtures/help`, then rerun the test without the variable.
`help_snapshots_cover_every_registered_command` keeps the file set and the
registry in step, so a new command cannot ship without a snapshot.
