#!/usr/bin/env bash
# The exit-code taxonomy (doc/cli/exit-codes.md): one deliberate trip to
# every code, including the ones the other scenarios don't reach naturally:
# a write blocked on another process's vault lock, a corrupt config, a
# damaged entry. run-all.sh separately checks every code 0-7 was asserted
# somewhere in the run.
source "$(dirname "$0")/../qa-env.sh"
scenario 10-exit-codes

new_vault alice v ""
put alice Entry e-value
put alice Other o-value
V="$(vault_dir alice v)"
VID="$(awk -F'"' '/^id *=/ {print $2; exit}' "$V/.gage/config.toml")"

step "0 success"
g alice -- vault list
expect_exit 0

step "1 conflict: a write waits ten seconds for the vault lock, then gives up"
LOCK="$(persona_home alice)/state/gage/locks/$VID.lock"
mkdir -p "$(dirname "$LOCK")"
python3 -c 'import fcntl,sys,time; f=open(sys.argv[1],"a"); fcntl.flock(f,fcntl.LOCK_EX); open(sys.argv[2],"w").close(); time.sleep(30)' \
    "$LOCK" "$QA_S/locked" &
HOLDER=$!
for _ in $(seq 50); do [[ -e "$QA_S/locked" ]] && break; sleep 0.1; done
start=$SECONDS
g alice -i "$(unlock alice)"$'\nx' -- insert Blocked --value-stdin
expect_exit 1
expect_true "it waited roughly ten seconds" test $((SECONDS - start)) -ge 9
step "reads don't take the lock"
g alice -i "$(unlock alice)" -- show Entry
expect_exit 0
kill "$HOLDER" 2>/dev/null; wait "$HOLDER" 2>/dev/null
step "after the holder dies, writes go through"
put alice Blocked x

step "2 usage"
g alice -- no-such-command
expect_exit 2
expect_err 'Usage'
g alice -- show
expect_exit 2
g alice -i "$(unlock alice)" -- init x --type svn --device a
expect_exit 2
g alice -i "$(unlock alice)" -- init x --device 'bad/name'
expect_exit 2

step "3 not-found"
g alice -i "$(unlock alice)" -- show NoSuchEntry
expect_exit 3
g alice -- vault info no-such-vault
expect_exit 3

step "4 ambiguous"
put alice "Entry two" e2
g alice -i "$(unlock alice)" -- show ntr
expect_exit 4

step "5 locked-or-auth: wrong passphrase, missing identity"
g alice -i "$(lines bad bad bad)" -- show Entry
expect_exit 5
mv "$(persona_home alice)/data/gage/identities" "$QA_S/identities.bak"
g alice -i "$(unlock alice)" -- show Entry
expect_exit 5
mv "$QA_S/identities.bak" "$(persona_home alice)/data/gage/identities"

step "6 internal: an unusable editor"
g alice -i "$(unlock alice)" -e EDITOR=/nonexistent/editor -- edit Entry
expect_exit 6

step "1 conflict: a damaged entry file"
F="$(ls "$V/entries" | head -1)"
cp "$V/entries/$F" "$QA_S/entry.bak"
printf 'garbage' >"$V/entries/$F"
g alice -i "$(unlock alice)" -- ls
# crypt.rs maps CorruptCiphertext to Conflict on purpose; doc/cli/exit-codes.md
# doesn't list it under 1 yet (a doc gap, not a behavior bug).
expect_exit 1
expect_err 'corrupt or truncated'
expect_no_err 'panicked'
cp "$QA_S/entry.bak" "$V/entries/$F"

step "a malformed global config is an error, never a panic"
C="$(persona_home alice)/config/gage/config.toml"
cp "$C" "$QA_S/config.bak"
printf 'this is = = not toml\n' >"$C"
g alice -- vault list
expect_exit 6
expect_no_err 'panicked'
cp "$QA_S/config.bak" "$C"

step "a committed config this gage refuses (wrong format_version)"
cp "$V/.gage/config.toml" "$QA_S/vconfig.bak"
sed -i.tmp -E 's/^format_version *= *[0-9]+/format_version = 999/' "$V/.gage/config.toml"
g alice -i "$(unlock alice)" -- show Entry
expect_exit 1
cp "$QA_S/vconfig.bak" "$V/.gage/config.toml"
rm -f "$V/.gage/config.toml.tmp"

step "7 unreachable"
R="$(bare_remote v)"
g alice -- git set-remote v "$QA_S/remotes/gone.git"
g alice -- pull
expect_exit 7

finish
