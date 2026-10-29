# ADR-0003: Advisory lock via `std::fs::File::lock`

- Status: Accepted
- Origin: Q-FILE-LOCK ([entry](../archive/rust-port/open-questions.md#q-file-lock))

## Context

Every write holds a per-vault advisory lock across its read-modify-commit
sequence. A primitive is needed on Linux, macOS and Windows.

## Decision

Use `std::fs::File::lock`/`try_lock` on every platform. No `fs4`.

Contended behavior: poll every 20 ms up to a caller-given timeout, then
return a typed `Contended { path, holder }` error. A zero timeout tries
once. The OS releases the lock when the holder exits, including on kill.

## Consequences

- On Windows a contender may be unable to read the holder's `pid=`/`since=`
  record from the locked range. The record is best-effort; tests assert
  holder detail on Unix only and, on Windows, only that a contended error is
  returned.
- Lock semantics are per open file description, so exclusion is proven with
  child processes, never threads.
