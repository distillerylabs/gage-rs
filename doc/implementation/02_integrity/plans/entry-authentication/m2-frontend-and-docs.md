[index](index.md) · [M0](m0-library-mac-and-v3.md) · [M1](m1-membership-and-sync.md) · **M2**

# M2 — Frontend & docs: CLI errors, mobile FFI, docs

> Recommended model: **Sonnet**. Well-specified, pattern-following work once
> M0/M1 fix the behavior — mapping an error, crossing the bindings, writing
> docs. Review the FFI change carefully, as it touches secret-adjacent paths.

## Goal

The CLI and the mobile bindings surface the new verification result cleanly,
and the docs describe the guarantee and its limits so the security model
matches the code.

## Depends on

M1 (every path authenticates; the `Error::Unauthenticated` exists). Branch off
the M1 branch.

## Design references

- `crates/gage-cli/src/exitcode.rs` — maps `ErrorKind` to exit codes 0–7;
  decide which code `Unauthenticated` maps to (it is a conflict-like integrity
  failure, not a usage error).
- `crates/gage-cli/src/` command handlers that read entries (`show`, `ls`,
  session index) — the message a user sees when an entry fails to verify.
- `crates/gage-ffi/` and [ADR-0063](../../../../decisions/0063-what-crosses-the-bindings.md)
  — the verification result and the new error crossing into Swift/Kotlin.
- `doc/design/security-model.md`, `SECURITY.md`, `doc/cli/` — where the new
  guarantee and its scope are documented.
- `tests/fixtures/help/` — update only if the command surface changes.

## Decisions to make first

1. **Exit code / `ErrorKind` for `Unauthenticated`.** Resolved: `Conflict`,
   exit `1`, the kind M0 gave it. It is an integrity failure like a corrupt
   ciphertext, which is also `Conflict`, and a script can tell it apart from
   "wrong key" (`5`) and "not found" (`3`). A human has to decide what to do
   about the file, which is what `conflict` means.
2. **What a failed verification says to the user.** Resolved: one bad entry
   fails the whole command, on `show` and on everything that decrypts many
   (`ls`, `search`, the session index, and so resolving any query), exactly
   as a corrupt entry does today. Nothing is skipped: a listing that silently
   lost a row looks complete when it isn't (the rule `history --decrypt`
   already follows), and a forged entry could otherwise hide a real one's
   absence. The message is the library's own, `entry <id> failed
   authentication: <why>`: the id is what a person needs to go and look, and
   nothing from the rejected payload appears in it. The way out is `git` in
   the vault's directory, since gage can't address the entry either; the CLI
   docs say so.
3. **FFI shape.** Resolved: no new case. A verification failure crosses as
   `GageError::Conflict` with the library's message unchanged (ADR-0054) and
   no remedy, like every other failure keyed by `ErrorKind` (ADR-0063). The
   message holds an id and a reason, never payload bytes.

## Tasks (tests first)

- [x] **Exit code.** Test: a command that hits an unverifiable entry exits
      with the chosen code and a clear message.
- [x] **`ls` / index behavior.** Test: the decided behavior (fail vs
      report-and-skip) for one unverifiable entry among many.
- [x] **FFI.** Test: the binding surfaces the verification failure as its
      typed error, message free of secrets.
- [x] **Docs.** Update `security-model.md` (the guarantee, and the
      out-of-scope limits: intra-recipient, removed-member window),
      `SECURITY.md` scope, and `doc/cli/` where entry integrity is described.
      No code; docs describe shipped behavior.
- [x] **Help snapshots.** Update `tests/fixtures/help/` only if the surface
      changed; otherwise confirm they are unchanged.

## Definition of done

- All tasks green; `make lint` and `make test` clean on all three platforms;
  `make qa` clean if the CLI surface changed.
- The security model and SECURITY.md describe the authentication guarantee and
  its limits; the CLI and FFI report a verification failure clearly.
- Stop for review.
