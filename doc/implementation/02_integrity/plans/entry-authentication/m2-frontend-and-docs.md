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
- `crates/gage-ffi/` and [ADR-0063](../../../decisions/0063-what-crosses-the-bindings.md)
  — the verification result and the new error crossing into Swift/Kotlin.
- `doc/design/security-model.md`, `SECURITY.md`, `doc/cli/` — where the new
  guarantee and its scope are documented.
- `tests/fixtures/help/` — update only if the command surface changes.

## Decisions to make first

1. **Exit code / `ErrorKind` for `Unauthenticated`.** Likely `Conflict`
   (exit-code-wise an integrity failure, like a corrupt ciphertext), so a
   script can tell it apart from "wrong key" and "not found". Confirm.
2. **What a failed verification says to the user.** A single rejected entry on
   `show`; and on `ls` / session-index build (which decrypts many) — does one
   bad entry fail the whole listing or is it reported and skipped? Decide and
   keep it consistent with how a corrupt entry is handled today.
3. **FFI shape.** Confirm the binding reports a verification failure as a
   typed error per ADR-0063, with nothing secret in the message.

## Tasks (tests first)

- [ ] **Exit code.** Test: a command that hits an unverifiable entry exits
      with the chosen code and a clear message.
- [ ] **`ls` / index behavior.** Test: the decided behavior (fail vs
      report-and-skip) for one unverifiable entry among many.
- [ ] **FFI.** Test: the binding surfaces the verification failure as its
      typed error, message free of secrets.
- [ ] **Docs.** Update `security-model.md` (the guarantee, and the
      out-of-scope limits: intra-recipient, removed-member window),
      `SECURITY.md` scope, and `doc/cli/` where entry integrity is described.
      No code; docs describe shipped behavior.
- [ ] **Help snapshots.** Update `tests/fixtures/help/` only if the surface
      changed; otherwise confirm they are unchanged.

## Definition of done

- All tasks green; `make lint` and `make test` clean on all three platforms;
  `make qa` clean if the CLI surface changed.
- The security model and SECURITY.md describe the authentication guarantee and
  its limits; the CLI and FFI report a verification failure clearly.
- Stop for review.
