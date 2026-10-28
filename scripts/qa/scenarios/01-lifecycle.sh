#!/usr/bin/env bash
# Vault lifecycle on one machine: init (with and without a recovery key, with
# a remote), a second vault, the default vault, -u, info, list and remove.
source "$(dirname "$0")/../qa-env.sh"
scenario 01-lifecycle

step "init writes the recovery key to a file, publishes to the remote"
R="$(bare_remote personal)"
new_vault alice personal "$R" --recovery-key-out "$QA_S/alice-personal.rk"
expect_out 'public key age1'
expect_out 'published "personal"'
expect_true "recovery key file exists" test -s "$QA_S/alice-personal.rk"
expect_true "recovery key file is AGE-SECRET-KEY" grep -q '^AGE-SECRET-KEY-' "$QA_S/alice-personal.rk"
expect_true "remote has the init commit" git -C "$R" log --oneline -1 main

step "the vault tree has the documented layout and nothing secret"
V="$(vault_dir alice personal)"
for f in .gage/config.toml .age-recipients .gitattributes .gitignore; do
    expect_true "vault has $f" test -f "$V/$f"
done
expect_true "identity lives outside the vault" test -d "$(persona_home alice)/data/gage/identities"
expect_true "no identity inside the vault tree" bash -c "! grep -rqs AGE-SECRET-KEY '$V'"

step "init refuses a name that is already registered"
g alice -i "$(lines "$(pw alice)" "$(pw alice)")" -- init personal --device alice-laptop --no-recovery-key
expect_exit 1

step "init without a terminal refuses to show a recovery key"
g alice -i "$(lines "$(pw alice)" "$(pw alice)")" -- init needs-tty --device alice-laptop
expect_exit 2
expect_err 'terminal'

step "a second, local-only vault"
new_vault alice work ""
g alice -- vault list
expect_exit 0
expect_out 'personal'
expect_out 'work'

step "the first vault is the default; set-default changes it"
put alice Only-In-Personal p-value -u personal
g alice -- vault set-default work
expect_exit 0
put alice Only-In-Work w-value
g alice -i "$(unlock alice)" -- show Only-In-Work
expect_exit 0
expect_out '^w-value$'
g alice -i "$(unlock alice)" -- show Only-In-Personal
expect_exit 3
g alice -i "$(unlock alice)" -- show Only-In-Personal -u personal
expect_exit 0
expect_out '^p-value$'

step "vault info reports type, method, recipients and remote"
g alice -- vault info personal
expect_exit 0
expect_out 'git'
expect_out 'passphrase'
# vault info takes a vault name, not -u.
g alice -- vault info -u personal
expect_exit 2

step "unknown vault is not-found"
g alice -- vault info nope
expect_exit 3
g alice -- vault set-default nope
expect_exit 3

step "vault remove forgets the vault but leaves its files"
g alice -i "$(lines n)" -- vault remove work
expect_exit 0
g alice -- vault list
expect_no_out '^\*? *work'
expect_true "work's files are still on disk" test -d "$(vault_dir alice work)/.gage"

finish
