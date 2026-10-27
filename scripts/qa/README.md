# Local QA harness

Black-box tests of the `gage` binary as several people on separate
machines, sharing vaults through git, entirely offline. This complements
the in-process suite (`make test`): it runs the real binary with real
prompts, real git remotes and real terminals, the way users do.

```
make qa                       # build, run every scenario, check coverage
make qa ARGS="04 06"          # only scenarios whose names start with these
scripts/qa/run-all.sh 04 06   # the same without make
```

The whole catalog takes a few minutes. The summary reports assertion
counts, which commands from Go's root help were never run, which exit
codes 0-7 were never asserted, and whether the real `~/.config/gage`,
`~/.local/share/gage` and `~/.local/state/gage` were left untouched. It
exits non-zero if any of those fail.

## How it isolates

- **A persona is a machine.** `alice`, `bob` and so on each get their own
  `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_STATE_HOME` under
  `$QA/s/<scenario>/<persona>/`. Every gage process runs under `env -i`
  with only those, plus `PATH`, `TMPDIR` and a fake `$EDITOR`.
- **Remotes are bare repos on local paths**, so no token or network is
  involved. An "unreachable" remote is a path that doesn't exist.
- **Prompts are answered from stdin.** With piped stdin, gage reads
  passphrases and confirmations as lines. The few prompts that refuse
  piped input by design run on a real pty through `expect` (`tty.exp`):
  the recipient-change review, the conflict menu, the recovery key shown
  on screen, and the interactive session. Without `expect` installed,
  those assertions are recorded as SKIP.

## Files

| File | What it is |
|---|---|
| `qa-env.sh` | The library: `scenario`, `step`, `g`/`gtty`/`as` runners, `expect_*` assertions, helpers (`new_vault`, `put`, `unlock`, `bare_remote`) |
| `tty.exp` | Runs a command on a pty and answers a scripted dialogue |
| `editor-sed.sh` | A non-interactive `$EDITOR` driven by `QA_EDIT_EXPR` |
| `scenarios/NN-*.sh` | The catalog. Each runs standalone |
| `run-all.sh` | Runs the catalog and checks the run as a whole |

Each run writes the following under `$QA`, a fresh temp directory unless
you set one:

- `results.tsv`: one row per assertion.
- `coverage.tsv`: one row per command run.
- `summary.txt`
- `s/<scenario>/log/transcript.txt`: every command with its stdin, exit
  code and output.

Passphrases are redacted from transcripts. The throwaway recovery keys and
tokens the scenarios create are not.

## Adding a scenario

```bash
#!/usr/bin/env bash
source "$(dirname "$0")/../qa-env.sh"
scenario 13-my-flow

R="$(bare_remote team)"
new_vault alice team "$R"
put alice Secret v1

step "bob can't read before he is added"
g bob -- clone "$R" --device bob-laptop
g bob -i "$(unlock bob)" -- show Secret
expect_exit 5

finish
```

- Assert the documented behavior and cite the doc in a comment when it is
  non-obvious. When the docs are silent, pin today's behavior and say so.
- Status lines go to stdout. Prompts, warnings and errors go to stderr.
- With piped stdin, prompts take lines in the order the command asks,
  which is not always unlock first. Read the transcript when a line seems
  ignored.
- To get a divergence, take one persona offline (`git set-remote` to a
  missing path) while both write. Every unlock fetches and fast-forwards.

## CI

`.github/workflows/qa.yml` runs `make qa` on Ubuntu and macOS for PRs
whose base is `main`, and for pushes to `main`. A stack's intermediate PRs
into other branches skip it. It is advisory, not a required check, while
its stability on Linux is established. Windows isn't covered yet (#104).

`make qa` sets `QA_REQUIRE_TTY=1`, so a run where `expect` is missing fails
instead of quietly recording the pty steps as SKIP. The job writes
`summary.txt` into the job summary, and on failure uploads the whole `$QA`
directory as the `qa-Linux`/`qa-macOS` artifact. Start with
`summary.txt`, then the failing scenario's `log/transcript.txt`.

## The QA agent

`.claude/agents/gage-qa.md` defines a Claude Code subagent that runs this
catalog and then explores edge cases in the same sandbox, writing
`$QA/report.md` with findings and repro transcripts. Ask Claude Code to
"use the gage-qa agent" (optionally: "on scenarios 04-06", "focus on
sync"). It never edits product code.
