# ADR-0044: No offline enrollment path

- Status: Accepted
- Origin: D-ENROLL-PRINT-ONLY ([design doc](../design/gage-cli-init-design.md))

## Decision

`--print-only` and `--request-file` do not exist. An offline path (print
the sealed request, let the approver read it from a file) forks nearly every
rule: it needs an armored wire format, has no filename and so no expiry or
id check, no commit so nothing prunes it, and no file to delete on approval
so the one-shot property stops holding.

## Consequences

For a remote this device cannot write to, or air-gapped transfer, the
answer is the manual path: `gage identity add`, carry the public key,
`gage recipient add` elsewhere. That path gives up the authentication
enrollment provides, and the trust-boundary documentation says so.
Accepting a request from a file later would be a new input source, not a
redesign, since the sealed blob is unchanged.
