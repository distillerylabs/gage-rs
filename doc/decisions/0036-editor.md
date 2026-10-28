# ADR-0036: `edit`: finding `$EDITOR` and the scratch file

- Status: Accepted
- Origin: Q-EDITOR ([entry](../archive/rust-port/open-questions.md#q-editor))

## Decision

- `$EDITOR` is trimmed and split on whitespace; the first word is resolved
  on `PATH` with `which` (in `gage-cli` only; it handles Windows
  `PATHEXT`) **before** any scratch file exists. Unset, blank or unresolvable
  is an error with no file written.
- Scratch directory: on Linux the first of `$XDG_RUNTIME_DIR` and
  `/dev/shm` that `statfs` reports as tmpfs; elsewhere
  `std::env::temp_dir()`. The file is created exclusively with an
  unpredictable name, 0600 on Unix, and on every exit path overwritten with
  zeros at its length and unlinked, best-effort.
- On Unix the handle the file was written through stays open across the
  editor and is zeroed too, so an editor that saves by renaming a new file
  over the path cannot leave the original inode un-zeroed. Windows closes it
  before the editor runs, since editors there may refuse a file another
  process holds.
- `$EDITOR` reaches the CLI through `Io::editor` (set by `main`), since
  `set_var` is `unsafe` and in-process tests share an environment. Tests
  fake the editor by re-executing the test binary, with the action in a
  config file named on the editor's command line.
