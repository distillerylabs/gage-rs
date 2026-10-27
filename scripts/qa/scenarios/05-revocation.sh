#!/usr/bin/env bash
# Revocation: alice removes bob with --reencrypt. Bob loses access to the
# current entries (old ciphertext in git history is out of reach of any
# re-encryption, as documented); carol, still a recipient, is asked to review
# the shrunken list before her next write. Plus the refusals: no
# --reencrypt, the last recipient, this device's own key.
source "$(dirname "$0")/../qa-env.sh"
scenario 05-revocation

R="$(bare_remote team)"
new_vault alice team "$R"
put alice Old old-value

# bob and carol each clone, register a key, and are added by alice.
for p in bob carol; do
    g "$p" -- clone "$R" --device "$p-laptop"
    g "$p" -i "$(lines "$(pw "$p")" "$(pw "$p")")" -- identity add --device "$p-laptop"
    key="$(grep -oE 'age1[0-9a-z]{58}' <<<"$OUT$ERR" | head -1)"
    g alice -i "$(unlock alice)" -- recipient add "$key" --device "$p-laptop"
    expect_exit 0
done
for p in bob carol; do
    g "$p" -- pull
    g "$p" -i "$(unlock "$p")" -- show Old
    expect_out '^old-value$'
done

step "recipient add's removed --reencrypt flag is a usage error"
g alice -i "$(unlock alice)" -- recipient add age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq --device x --reencrypt
expect_exit 2

step "recipient remove requires --reencrypt"
g alice -i "$(unlock alice)" -- recipient remove bob-laptop
expect_exit 2
g alice -- recipient list
expect_out '^bob-laptop '

step "removing bob re-encrypts every entry without his key"
g alice -i "$(unlock alice)" -- recipient remove bob-laptop --reencrypt
expect_exit 0
expect_err 'revokes future access only'
g alice -- recipient list
expect_no_out '^bob-laptop '
g alice -- recipient verify
expect_exit 0
put alice New new-value

step "bob pulls and can read neither old nor new entries at HEAD"
g bob -- pull
expect_exit 0
g bob -i "$(unlock bob)" -- show Old
expect_exit 5
expect_no_out 'old-value'
g bob -i "$(unlock bob)" -- show New
expect_exit 5
expect_no_out 'new-value'

step "bob can't write either"
g bob -i "$(unlock bob)"$'\nsneaky' -- --yes insert Sneaky --value-stdin
expect_exit 5

step "history: bob still decrypts the revisions he could always read"
# Documented: "removing a recipient revokes future access only". This
# records today's behavior; it must never include new-value.
g bob -i "$(unlock bob)" -- history --decrypt New
expect_exit 5
expect_no_out 'new-value'
g bob -i "$(unlock bob)" -- history --decrypt Old
expect_no_out 'new-value'

step "carol's next write asks her to review the removal"
g carol -- pull
g carol -i "$(unlock carol)"$'\nv' -- insert FromCarol --value-stdin
expect_exit 1
gtty carol "$(dialogue "$PROMPT_PW" "$(pw carol)" 'Value for' carol-v 'trust this recipient list\?' y)" -- insert FromCarol
expect_exit 0
expect_out 'removed|bob-laptop'
g carol -i "$(unlock carol)" -- show New
expect_out '^new-value$'

step "removing this device's own key asks; --yes does not answer"
g alice -- pull
g alice -i "$(unlock alice)" -- --yes recipient remove alice-laptop --reencrypt
expect_exit 5
expect_err 'could not be confirmed'
g alice -- recipient list
expect_out '^alice-laptop '

step "removing an unknown recipient is not-found"
g alice -i "$(unlock alice)" -- recipient remove nobody-laptop --reencrypt
expect_exit 3

step "the last recipient can't be removed"
new_vault dave solo ""
g dave -i "$(unlock dave)" -- recipient remove dave-laptop --reencrypt
expect_exit 1
expect_err 'at least one recipient'
g dave -- recipient list
expect_out '^dave-laptop '

finish
