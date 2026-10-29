# ADR-0037: Exit codes 0–7

- Status: Accepted
- Origin: the `exitcode` taxonomy; archived Q-COMPAT ([entry](../archive/rust-port/open-questions.md#q-compat))

## Decision

Every command exits with one of eight codes, so scripts and CI can branch on
*why* a command failed. The numbers are a contract.

| Code | Name | Meaning |
|---|---|---|
| 0 | success | The command did what it was asked |
| 1 | conflict | Something needs a human to resolve |
| 2 | usage | The command line itself was invalid |
| 3 | not-found | A name or query resolved to nothing |
| 4 | ambiguous | A query matched several candidates and nobody could be asked |
| 5 | locked-or-auth | An unlock failed, or a git host refused gage |
| 6 | internal | An unexpected error that fits none of the above |
| 7 | unreachable | A remote could not be reached at all |

`unreachable` is distinct from `conflict`: nothing is wrong with either
side's state, there is nothing to resolve, and the command is worth retrying.
The full list of what maps to each code is [doc/cli/exit-codes.md](../cli/exit-codes.md).

## Consequences

Library errors map to codes in `gage-cli`, not in the library. Every error
message goes to stderr, prefixed `gage:` exactly once.
