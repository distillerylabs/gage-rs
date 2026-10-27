#!/usr/bin/env bash
# Two people share a vault by public key: bob clones and runs identity add,
# alice runs recipient add, bob pulls and can read. Covers clone, identity
# add/list, recipient add/list/verify, push/pull and the recipient-change
# confirmation (with and without --yes).
source "$(dirname "$0")/../qa-env.sh"
scenario 03-share-by-key

R="$(bare_remote team)"
new_vault alice team "$R"
put alice Shared s3cret

step "bob clones, but cloning grants no access"
g bob -- clone "$R" --device bob-laptop
expect_exit 0
expect_out 'cloned "team"'
expect_out 'no identity for "team"'
g bob -i "$(unlock bob)" -- show Shared
expect_exit 5
expect_no_out 's3cret'

step "clone refuses a name already registered"
g bob -- clone "$R" --device bob-laptop
expect_exit 1

step "bob registers an identity and gets a public key"
g bob -i "$(lines "$(pw bob)" "$(pw bob)")" -- identity add --device bob-laptop
expect_exit 0
BOB_KEY="$(grep -oE 'age1[0-9a-z]{58}' <<<"$OUT$ERR" | head -1)"
expect_true "identity add printed an age public key" test -n "$BOB_KEY"
g bob -- identity list
expect_exit 0
expect_out 'bob-laptop'

step "bob still can't read until alice adds him"
g bob -i "$(unlock bob)" -- show Shared
expect_exit 5

step "recipient add validates the key and requires --device"
g alice -i "$(unlock alice)" -- recipient add age1notakey --device bob-laptop
expect_exit 2
g alice -i "$(unlock alice)" -- recipient add "$BOB_KEY"
expect_exit 2

step "alice adds bob's key: re-encrypts and pushes"
g alice -i "$(unlock alice)" -- recipient add "$BOB_KEY" --device bob-laptop
expect_exit 0
g alice -- recipient list
expect_exit 0
expect_out "^bob-laptop +$BOB_KEY"
expect_out '^alice-laptop '
g alice -- recipient verify
expect_exit 0

step "recipient add refuses a key that is already a recipient"
g alice -i "$(unlock alice)" -- recipient add "$BOB_KEY" --device bob-again
expect_exit 1

step "bob pulls and reads; the pull warns that recipients changed"
g bob -- pull
expect_exit 0
g bob -i "$(unlock bob)" -- show Shared
expect_exit 0
expect_out '^s3cret$'

step "bob's first write asks to confirm the new recipient list"
# No terminal: refused before reading anything (Q-RECIPIENT-PROMPT), so a
# piped answer can't be spent on it. Nothing is written.
g bob -i "$(unlock bob)"$'\nv' -- insert FromBob --value-stdin
expect_exit 1
expect_err 'nobody is at a terminal'
g bob -i "$(unlock bob)" -- show FromBob
expect_exit 3

step "on a terminal the review names the new key; declining writes nothing"
gtty bob "$(dialogue "$PROMPT_PW" "$(pw bob)" 'Value for' tty-v 'trust this recipient list\?' n)" -- insert FromBob
expect_exit 1
expect_out 'Recipients for "team" changed'
expect_out "$BOB_KEY|bob-laptop"
expect_no_out "$(pw bob)"
g bob -i "$(unlock bob)" -- show FromBob
expect_exit 3

step "confirming on a terminal writes"
gtty bob "$(dialogue "$PROMPT_PW" "$(pw bob)" 'Value for' tty-v 'trust this recipient list\?' y)" -- insert FromTty
expect_exit 0
g bob -i "$(unlock bob)" -- show FromTty
expect_out '^tty-v$'
# Confirming recorded the list, so --yes below has nothing to answer.
# Re-arm the question by forgetting bob's trust cache.
rm -rf "$(persona_home bob)"/state/gage/*/trust.toml

step "--yes answers only the recipient-change question"
g bob -i "$(unlock bob)"$'\nbob-value' -- --yes insert FromBob --value-stdin
expect_exit 0
g bob -i "$(unlock bob)" -- show FromBob
expect_out '^bob-value$'

step "after confirming once, later writes don't ask"
put bob FromBob2 v2

step "alice pulls bob's writes and can read them"
g alice -- pull
expect_exit 0
g alice -i "$(unlock alice)" -- show FromBob
expect_exit 0
expect_out '^bob-value$'
g alice -i "$(unlock alice)" -- ls
expect_out 'FromBob2 .*bob-laptop$'

step "push with nothing new is a no-op; pull is up to date"
g alice -- push
expect_exit 0
g alice -- pull
expect_exit 0

step "a tampered .age-recipients is caught by verify"
V="$(vault_dir alice team)"
cp "$V/.age-recipients" "$QA_S/recipients.bak"
echo "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq" >>"$V/.age-recipients"
g alice -- recipient verify
expect_true "verify fails on a mismatch" test "$CODE" -ne 0
cp "$QA_S/recipients.bak" "$V/.age-recipients"
g alice -- recipient verify
expect_exit 0

finish
