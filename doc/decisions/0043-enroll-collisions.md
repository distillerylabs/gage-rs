# ADR-0043: Device names are labels; the public key is the identity

- Status: Accepted
- Origin: D-ENROLL-COLLISIONS ([design doc](../design/gage-cli-init-design.md))

## Decision

A device name is a label. The public key is the identity. Two machines both
called `macbook-pro` is ordinary, so:

- **At enroll time, this device is already a recipient** (by public key, read
  from the local record without an unlock, [ADR-0013](0013-orphan-identity-by-pubkey.md)):
  success that does no work. Nothing is generated, sealed or published, and
  gage says this device can already read the vault. The key question comes
  before the name question, so the user is never told to pass `--device` to
  mint a second identity for access they already have.
- **At enroll time, a different recipient has the name**: refused with an
  error that names the fix, `--device NAME`. If the public key record is
  absent, the name check is the fallback.
- **Between enroll and approve**, `approve --device NAME` relabels the
  request, for exactly one request per run. The approver owns the recipient
  list. Without it, a request would be permanently unapprovable.
- **Request's key is already a recipient**: approval is a success that does
  no work (clear the request, report it, re-encrypt nothing).

Relabeling does not weaken authentication: the code authenticates the key;
the name was advisory metadata. The opened request stays immutable, and the
chosen label travels separately.

## Consequences

Silent auto-suffixing (`macbook-pro-2`) was rejected as contradicting the
posture of every other decision here.
