# ADR-0080: A token is checked by asking for both ref advertisements

- Status: Accepted
- Origin: [#232](https://github.com/distillerylabs/gage-rs/issues/232), from gage-ios's Settings and Join screens

## Context

A token was only tested by the next fetch or push that used it. gage-ios's
Settings screen could only say a new token "will be tested at the next
sync", and a bad one came back later as Access refused. Join had a worse
case: a token that can read but not push clones fine and is refused only at
`enroll`'s push, after the device has created its identity.

A check that answers "can this token push?" must not push. A vault's remote
is shared, and a test commit or ref left on it is a change every member
pulls.

## Decision

**The check asks the remote's fetch service for its ref advertisement, then
its push service, and stops there.** That is as far as `git push --dry-run`
goes on the network, and it is where hosts check write access: GitHub and
GitLab refuse the push service's advertisement to a token without write
access. Both connections go through a scratch repository, like the clone's
branch listing. Nothing is downloaded or sent, and the vault's own
repository is never opened. One timeout covers both connections.

**A fetch-side refusal is the error, and a push-side auth refusal is an
answer.** `gitrepo::check_access` returns `Access::ReadWrite` or
`Access::ReadOnly`. A token the fetch service refuses fails the way any
fetch does (`syncerr` auth with `Remedy::AuthLogin`, or unreachable), and so
does a push side that fails for any reason other than auth. "Read-only" is
typed, not an error, because what it means depends on the screen: Settings
can save it with a warning, and Join should refuse it.

**A candidate token is tested in place of the stored one and is never
stored.** `remotecheck::check_remote(dirs, url, token)` and
`Vault::check_remote(token)` take an optional token. With one, it goes only
where a stored token could go (`token_host`: HTTPS, or loopback HTTP), so
the check can't send a token anywhere a sync wouldn't (ADR-0075). Without
one, whatever a fetch would offer is tested. Testing before saving means a
bad token never replaces a good one, and nothing gives the token back
(ADR-0063).

## Consequences

- A host that checks write access only when a pack arrives, not at the
  advertisement, reports a read-only token as `ReadWrite`. The check can't
  rule that out without pushing, which it never does. The first real push
  still reports the refusal, with its access remedy (ADR-0079).
- An anonymous check of a public repository is `ReadOnly`, the same answer
  a read-only token gets. With a token, `ReadOnly` means the token.
- The check takes no vault lock and reads only the origin's URL, so it can
  run beside a write.
- `gage auth login` runs the check against the current vault's origin when
  it is on the token's host. A refused token is not stored (exit 5), with no
  "run `gage auth login`" hint, since rerunning with the same token can't
  help. A read-only token is stored with a warning. An unreachable host
  stores the token unchecked, since logging in is local. A token for a host
  no current vault uses is stored unchecked, and the output says so.
