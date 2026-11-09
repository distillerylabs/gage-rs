# ADR-0075: A token travels only over HTTPS or to this machine

- Status: Accepted
- Origin: Small hardening pass ([#220](https://github.com/distillerylabs/gage-rs/issues/220))
- Amends: [ADR-0020](0020-remote-auth.md), which offered the token to any HTTP(S) remote on its host

## Context

A stored token is offered when a host asks for credentials, and that included
`http://`, so anyone on the network path could read it.

## Decision

The credentials callback refuses to hand a token to a non-loopback `http://`
URL, and the error says to use the HTTPS URL.

- **In the callback, not when the method is chosen.** It is the one place that
  sees the URL a redirect ended up at, so an `https` remote redirecting to
  `http` is covered. An `http://` remote that never asks for credentials still
  works anonymously.
- **Loopback stays allowed**: `localhost` and loopback addresses. That traffic
  doesn't leave the machine, and a local test server needs it.
- **No opt-in flag.** Nothing supported needs one yet.
- **`init --remote` doesn't ask for a token it wouldn't send.**

## Consequences

A self-hosted server reachable only over `http://` on a network can't use a
stored token from gage. It needs HTTPS.
