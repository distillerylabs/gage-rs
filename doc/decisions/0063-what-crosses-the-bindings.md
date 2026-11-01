# ADR-0063: Secrets cross the bindings on request; remedies cross as data

- Status: Accepted
- Origin: A1c, bindings for join + read ([#181](https://github.com/distillerylabs/gage-rs/issues/181))

## Context

[ADR-0060](0060-mobile-bindings.md) says how types cross into Swift and
Kotlin. A1c is the first time secret values cross, and the first time an
app needs to act on a failure rather than only show it.

What the bindings can protect is limited by how UniFFI moves data. A value
coming in is lifted into a Rust `String` without a copy, so Rust can wipe
it. A value going out becomes a `RustBuffer` that the foreign side reads and
UniFFI then frees without wiping, with no Rust code in between.

## Decision

**A secret crosses only when the app asks for that one secret.** A call that
returns entry data returns metadata and field names. Values come from
`reveal`, one at a time. No record that crosses holds an entry's value, a
field's value or an unknown field.

**The bindings wipe every copy they hold.** Each decrypted entry is wiped
when the call ends, whether or not it is revealed, and each secret coming in
is wiped after use.

**What goes out is the app's to protect, and the docs say so plainly.** The
Swift or Kotlin string is out of reach, and one unwiped transfer buffer is
left behind for each secret going out. We accept that rather than build a
custom transfer type, because the foreign string it would be copied into is
unwipeable anyway.

**A record holding a secret has a hand-written `Debug`** that redacts it, as
the library's own types do.

**A token goes in and never comes out.** The bindings store, list and remove
tokens; none of them returns one.

**A remedy crosses as a `Remedy` enum,** on `Warning` and on every
`GageError` case, mirroring the library's plus `Other`, which a variant added
to the library's `#[non_exhaustive]` enum maps to until the bindings name it.
No command text crosses; the app words it (ADR-0054).

## Consequences

- Revealing a secret leaves up to two unwiped copies in the app's process
  until the allocator reuses them. This limits what an app can promise about
  a memory dump; it doesn't change what is on disk.
- A later binding that returns entry data can't add a value to its record
  without changing this ADR. Writes in A2 send values inward, which can be
  wiped.
- A new library `Remedy` reaches apps as `Other` until the mapping is
  extended. An app treats `Other` as "no suggested action" and still has the
  message.
