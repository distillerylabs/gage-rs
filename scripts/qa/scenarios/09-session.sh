#!/usr/bin/env bash
# Session mode without a human: --stdin and --script with GAGE_PASSPHRASE
# (and without it), stopping at the first failing line with its location,
# refusing to create an identity, --yes for a whole run, the mutual
# exclusions. Then one interactive session on a real terminal.
source "$(dirname "$0")/../qa-env.sh"
scenario 09-session

R="$(bare_remote team)"
new_vault alice team "$R"
put alice Alpha a-value
put alice Beta b-value
PP="GAGE_PASSPHRASE=$(pw alice)"

step "--stdin runs lines with GAGE_PASSPHRASE, unlocking once"
g alice -e "$PP" -i "$(lines 'show Alpha' 'show Beta' 'ls')" -- --stdin
expect_exit 0
expect_out '^a-value$'
expect_out '^b-value$'
expect_out '^Alpha '
expect_true "the passphrase was asked for at most once" test "$(grep -c 'Enter passphrase' <<<"$ERR")" -le 1

step "writes in a script, then reads them back"
g alice -e "$PP" -i "$(lines 'generate Gamma -l 16' 'rename Gamma Delta' 'show Delta' 'rm Delta')" -- --stdin
expect_exit 0
expect_out '^.{16}$'

step "GAGE_PASSPHRASE is ignored by a one-shot command"
g alice -e "$PP" -- show Alpha
expect_exit 5

step "--stdin with no passphrase source refuses, naming the variable"
g alice -i "$(lines 'show Alpha')" -- --stdin
expect_exit 5
expect_err 'GAGE_PASSPHRASE'
expect_err '<stdin>:1:'

step "a wrong GAGE_PASSPHRASE fails once, no retries"
g alice -e GAGE_PASSPHRASE=wrong -i "$(lines 'show Alpha')" -- --stdin
expect_exit 5
expect_no_out 'a-value'

step "a failing line stops the run and is located; later lines don't run"
g alice -e "$PP" -i "$(lines 'show Alpha' 'show Nope' 'show Beta')" -- --stdin
expect_exit 3
expect_err '<stdin>:2:'
expect_out '^a-value$'
expect_no_out 'b-value'

step "an ambiguous query with nobody to pick fails, consuming no line"
put alice Alpha2 a2-value
g alice -e "$PP" -i "$(lines 'show Alph' 'show Beta')" -- --stdin
expect_exit 4
expect_no_out 'b-value'

step "--script FILE, with file:line locations"
S="$QA_S/cmds.gage"
lines 'show Alpha' 'show Missing' >"$S"
g alice -e "$PP" -- --script "$S"
expect_exit 3
expect_out '^a-value$'
expect_err 'cmds.gage:2:'
g alice -e "$PP" -- --script "$QA_S/no-such-file.gage"
expect_exit 2

step "a script can't create an identity: GAGE_PASSPHRASE is never a new key's"
g bob -- clone "$R" --device bob-laptop
g bob -e "GAGE_PASSPHRASE=$(pw bob)" -i "$(lines 'identity add --device bob-laptop')" -- --stdin
expect_exit 5
expect_err 'typed by a human'
expect_true "no identity file was written" bash -c "! ls '$(persona_home bob)'/data/gage/identities/*/* 2>/dev/null | grep -q ."

step "a script's write to a changed recipient list fails 1 without --yes"
g bob -i "$(lines "$(pw bob)" "$(pw bob)")" -- identity add --device bob-laptop
BOB_KEY="$(grep -oE 'age1[0-9a-z]{58}' <<<"$OUT$ERR" | head -1)"
# Bob looks at the vault before he is added: trust-on-first-use records
# the list he saw (doc/cli/identities-and-recipients.md), so the add below is
# a change. Without this look, his first write would trust the new list.
g bob -i "$(unlock bob)" -- show Alpha
expect_exit 5
g alice -i "$(unlock alice)" -- recipient add "$BOB_KEY" --device bob-laptop
g bob -- pull
g bob -e "GAGE_PASSPHRASE=$(pw bob)" -i "$(lines 'generate FromScript')" -- --stdin
expect_exit 1
g bob -e "GAGE_PASSPHRASE=$(pw bob)" -i "$(lines 'generate FromScript' 'show Alpha')" -- --stdin --yes
expect_exit 0
expect_out '^a-value$'

step "approve is not scriptable; pending and deny are"
g carol -- clone "$R" --device carol-laptop
g carol -i "$(lines "$(pw carol)" "$(pw carol)")" -- identity enroll --device carol-laptop
CODE_C="$(grep -oE 'GAGE(-[A-Z0-9]{4}){4}' <<<"$OUT$ERR" | head -1)"
g alice -- pull
g alice -e "$PP" -i "$(lines "recipient approve --code $CODE_C")" -- --stdin --yes
expect_exit 5
g alice -e "$PP" -i "$(lines 'recipient pending')" -- --stdin
expect_exit 0
expect_out '1 pending'

step "mutual exclusions are usage errors"
g alice -e "$PP" -i "" -- --stdin --script "$S"
expect_exit 2
g alice -e "$PP" -i "" -- --stdin show Alpha
expect_exit 2
g alice -- --yes
expect_exit 2

step "bare gage with stdin not a terminal prints help"
g alice --
expect_exit 0
expect_out 'Vault lifecycle'

step "an interactive session on a terminal: unlock once, Up recalls, lock, exit"
gtty alice "$(dialogue 'gage.*> ' 'show Alpha' "$PROMPT_PW" "$(pw alice)" 'gage.*> ' 'show Beta' \
    'gage.*> ' 'help' 'gage.*> ' 'lock' 'gage.*> ' 'show Beta' "$PROMPT_PW" "$(pw alice)" 'gage.*> ' 'exit')" --
expect_exit 0
expect_out 'a-value'
expect_out 'b-value'
expect_out 'insert'
expect_true "the passphrase was asked twice (once more after lock)" test "$(grep -c 'Enter passphrase' <<<"$OUT")" -eq 2
expect_no_out "$(pw alice)"

step "--yes is refused inside an interactive session"
gtty alice "$(dialogue 'gage.*> ' 'exit')" -- --yes
expect_exit 2

finish
