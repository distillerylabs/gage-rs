# Addressing entries

Entry files are named by random UUIDs that mean nothing, so there is no
path to type the way there is with `pass`. Instead `show`, `cat`, `edit`,
`rename` and `rm` take a **query**, matched against the decrypted titles
and the entries' ids. (`insert` and `generate` take a plain `<title>`:
they create an entry rather than address one.)

> **Status.** `mv` and `cp` arrive in a later phase of the
> [port plan](../implementation/00_gage-cli/plans/rust-port/index.md).

## Resolution order

A query is tried in this fixed order, stopping at the first stage that
matches anything:

1. Exact title (byte for byte: case and spaces count).
2. Title substring, ignoring case: `root` and `ROOT` both find `AWS root
   account`.
3. Exact id, in any spelling a UUID is written: hyphenated, without
   hyphens, braced, or as a `urn:uuid:`.
4. Id substring, ignoring case and hyphens on both sides. A prefix is one
   (the 8-character short id `ls` prints works), and so is a run in the
   middle that spans a hyphen.
5. Nothing at any stage: not found, exit `3`.

Only titles are searched, never descriptions or values: finding entries
by their contents is `search`'s job, not addressing's.

If a stage matches **more than one** entry, resolution stops there and
reports that stage's candidates. It never falls through to a later stage,
and never picks one for you.

Every title stage runs before any id stage on purpose. Entries are
addressed by title far more often than by an id nobody has memorized, and
a short title that is spellable in hex (`22`, `dead`, `beef`) can turn up
inside some other entry's random id. Checking titles first means a title
always names its own entry, and an id substring can never silently win
over it.

## An ambiguous query

A one-shot command has nobody to ask, so it does nothing, lists the
candidates on stderr with their full ids, and exits `4` (`ambiguous`):

```
$ gage show aws
gage: "aws" matches 2 entries; name one by its id:
  a03e5f88-9c1d-4f0e-8a51-6b2f0c7d9e14  AWS IAM backup admin
  4b9d7710-2e6a-4c8b-9f3d-1a5e7c0b8d26  AWS root account
```

Candidates are sorted by title, then id. Pass one of the ids (or enough of
it to be unique) to pick. Not found and ambiguous are different exit
codes (see [Exit codes](exit-codes.md)), so a script can tell "no such
entry" from "be more specific".

In a [session](session-mode.md) there is someone to ask, so the same
candidate list becomes a numbered picker, and the entry you pick is the
one the command uses. Same list, same order: the two modes differ only in
what they do with it.

## Enrollment request IDs reuse the same order

`recipient approve <ID>` and `recipient deny <ID>` address a pending
enrollment request rather than an entry, and they reuse this order rather
than inventing one: exact UUID, then part of the UUID, then a candidate
list (never a guess), then not found. The eight-character prefix
`recipient pending` shows is the "part of the UUID" case. An ambiguous id
lists each candidate's short id and expiry and exits `4`, in a session as
well as one-shot: there is no picker for requests, because `deny` deletes.

Two details are specific to requests. Matching runs against the **UUID
part of the filename only**. A pending file is named
`<uuid>-<expires-epoch>.age`, and a short numeric id like `1788` could
otherwise match one request's expiry and another's id, with `deny`
deleting whichever it hit. And there is no matching on device name: the
name is sealed inside the request, which is the whole point of the
filename scheme. Request ids are not offered by Tab completion. See
[Adding a device](enrollment.md#addressing-a-request-by-id).

## Why addressing needs an unlock

Titles live inside the encrypted entries, so resolving a query, even
listing entries, decrypts every one of them. A one-shot command pays that
cost each time it runs. A session pays it once per vault and resolves
from its [metadata index](session-mode.md#the-metadata-index) after that,
decrypting only the entry a query lands on.
