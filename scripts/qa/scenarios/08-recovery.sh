#!/usr/bin/env bash
# The recovery key: shown once on a terminal (and typed back), written to a
# file, verified, rotated, and used to rebuild a lost machine. alice's laptop
# is lost: a fresh machine (alice2) clones and runs recovery enroll
# --replaces, and the lost laptop's key stops opening anything new.
source "$(dirname "$0")/../qa-env.sh"
scenario 08-recovery

R="$(bare_remote team)"
RK="$QA_S/alice.rk"

step "init on a terminal shows the key once and asks for its tail back"
gtty alice "$(dialogue 'Choose a passphrase' "$(pw alice)" 'Confirm passphrase' "$(pw alice)" 'last [0-9]+ characters' '^RKTAIL')" \
    -e "QA_RK_SAVE=$RK" -- init team --device alice-laptop --remote "$R"
expect_exit 0
expect_out 'only time it will be shown'
expect_out 'AGE-SECRET-KEY-1'
expect_true "the harness copied the key off the screen" grep -q '^AGE-SECRET-KEY-1' "$RK"
if [[ "$CODE" == skip ]]; then
    # No expect: fall back to a file, so the rest still runs.
    rm -rf "$(persona_home alice)" "$R"
    R="$(bare_remote team)"
    new_vault alice team "$R" --recovery-key-out "$RK"
fi
put alice Secret s1
g alice -- recipient list
expect_out '^recovery-paper-key +age1'

step "a fumbled tail on the terminal fails 1 but the vault exists"
gtty alice "$(dialogue 'Choose a passphrase' "$(pw alice)" 'Confirm passphrase' "$(pw alice)" \
    'last [0-9]+ characters' nope 'try again|last [0-9]+ characters' nope 'try again|last [0-9]+ characters' nope)" \
    -- init fumbled --device alice-laptop
expect_exit 1
expect_out 'not confirmed'
g alice -- vault list
expect_out 'fumbled'

step "recovery verify: the stored key is a recipient; another key isn't"
g alice -i "$(cat "$RK")" -- recovery verify
expect_exit 0
g alice -i "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ" -- recovery verify
# Not valid bech32, so a usage error; a valid foreign key is 5 (below, after rotate).
expect_exit 2
g alice -i "not a key" -- recovery verify
expect_exit 2
expect_no_out 'not a key'
expect_no_err 'not a key'

step "recovery rotate retires the old key and writes the new one to a file"
g alice -- recipient list
OLD_PUB="$(awk '/^recovery-paper-key/ {print $2}' <<<"$OUT")"
g alice -i "$(unlock alice)" -- recovery rotate --recovery-key-out "$QA_S/alice-rotated.rk"
expect_exit 0
expect_true "the rotated key was written" grep -q '^AGE-SECRET-KEY-1' "$QA_S/alice-rotated.rk"
g alice -- recipient list
expect_no_out "$OLD_PUB"
g alice -i "$(cat "$RK")" -- recovery verify
expect_exit 5
g alice -i "$(cat "$QA_S/alice-rotated.rk")" -- recovery verify
expect_exit 0
mv "$QA_S/alice-rotated.rk" "$RK"

step "--recovery-key-out refuses an existing file or a path inside the vault"
g alice -i "$(unlock alice)" -- recovery rotate --recovery-key-out "$RK"
expect_exit 2
g alice -i "$(unlock alice)" -- recovery rotate --recovery-key-out "$(vault_dir alice team)/key.txt"
expect_exit 2
g alice -i "$(unlock alice)" -- recovery rotate
expect_exit 2

step "alice's laptop is lost: a new machine clones and recovers"
g alice2 -- clone "$R" --device alice-laptop
expect_exit 0
g alice2 -i "$(lines "$(cat "$RK")" "$(pw alice2)" "$(pw alice2)")" -- \
    recovery enroll --device alice-desktop --replaces alice-laptop --recovery-key-out "$QA_S/alice2.rk"
expect_exit 0
g alice2 -i "$(unlock alice2)" -- show Secret
expect_exit 0
expect_out '^s1$'
g alice2 -- recipient list
expect_out '^alice-desktop '
expect_no_out '^alice-laptop '

step "the used recovery key is retired; the replacement verifies"
g alice2 -i "$(cat "$RK")" -- recovery verify
expect_exit 5
g alice2 -i "$(cat "$QA_S/alice2.rk")" -- recovery verify
expect_exit 0

step "the lost laptop's key opens nothing written after"
put alice2 AfterRecovery r1
g alice -- pull
g alice -i "$(unlock alice)" -- show AfterRecovery
expect_exit 5
expect_no_out 'r1'

step "recovery enroll refusals"
g alice2 -i "$(lines "$(cat "$QA_S/alice2.rk")")" -- recovery enroll --device recovery-paper-key --no-new-recovery-key
expect_exit 2
g alice2 -i "$(lines "$(cat "$QA_S/alice2.rk")")" -- recovery enroll --replaces recovery-paper-key --no-new-recovery-key
expect_exit 2
g alice2 -i "$(lines "$(cat "$QA_S/alice2.rk")")" -- recovery enroll --no-new-recovery-key --recovery-key-out "$QA_S/x.rk"
expect_exit 2

finish
