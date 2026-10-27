#!/usr/bin/env bash
# Sharing an entry by moving or copying it between vaults with different
# recipients: alice's personal vault, and a team vault bob can read. After
# cp both copies exist; after mv only the destination's; bob reads what
# landed in team and nothing else. Plus the usage and not-found refusals.
source "$(dirname "$0")/../qa-env.sh"
scenario 07-cross-vault

RT="$(bare_remote team)"
new_vault alice personal "$(bare_remote personal)"
new_vault alice team "$RT"
g bob -- clone "$RT" --device bob-laptop
g bob -i "$(lines "$(pw bob)" "$(pw bob)")" -- identity add --device bob-laptop
BOB_KEY="$(grep -oE 'age1[0-9a-z]{58}' <<<"$OUT$ERR" | head -1)"
g alice -i "$(unlock alice)" -- recipient add "$BOB_KEY" --device bob-laptop -u team
expect_exit 0
put alice "Wifi" wifi-pass -u personal --field ssid=home
put alice "Bank" bank-pass -u personal

TWO="$(unlock alice "$(pw alice)")"

step "cp copies into the other vault and keeps the original"
g alice -i "$TWO" -- cp Wifi --to-vault team -u personal
expect_exit 0
g alice -i "$(unlock alice)" -- show Wifi -u personal
expect_out '^wifi-pass$'
g alice -i "$(unlock alice)" -- show Wifi -u team
expect_out '^wifi-pass$'
g alice -i "$(unlock alice)" -- show Wifi -u team --field ssid
expect_out '^home$'

step "bob reads the copy in team"
g bob -i "$(unlock bob)" -- show Wifi
expect_exit 0
expect_out '^wifi-pass$'

step "mv moves: gone from personal, present in team"
g alice -i "$TWO" -- mv Bank --to-vault team -u personal
expect_exit 0
g alice -i "$(unlock alice)" -- show Bank -u personal
expect_exit 3
g alice -i "$(unlock alice)" -- show Bank -u team
expect_out '^bank-pass$'
g bob -i "$(unlock bob)" -- show Bank
expect_out '^bank-pass$'

step "the moved entry got a ciphertext bob can open; personal never did"
expect_true "bob has no access to personal's remote content" \
    bash -c "! grep -rqs bank-pass '$(persona_home bob)'"

step "mv/cp refusals"
g alice -i "$TWO" -- mv Wifi -u personal
expect_exit 2
g alice -i "$TWO" -- mv Wifi --to-vault personal -u personal
expect_exit 2
g alice -i "$TWO" -- cp Wifi --to-vault nowhere -u personal
expect_exit 3
g alice -i "$TWO" -- cp NoSuchEntry --to-vault team -u personal
expect_exit 3

step "cp onto a title the destination already has is allowed (doc/cli/sharing.md)"
g alice -i "$TWO" -- cp Wifi --to-vault team -u personal
expect_exit 0
g alice -i "$(unlock alice)" -- show Wifi -u team
expect_exit 4

finish
