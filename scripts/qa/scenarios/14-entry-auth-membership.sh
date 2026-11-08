#!/usr/bin/env bash
# Entry authentication when membership changes (ADR-0066). Adding a
# recipient re-seals the vault's MAC key and does not rotate it; removing one
# (and recovery rotate / enroll --replaces, which drop a key) rotates it.
# Rotation is proved by contrast, not by parsing the key file: save an
# entry's file, change membership, put the saved file back. It is still
# encrypted to the people reading it, so a refusal can only come from the tag.
source "$(dirname "$0")/../qa-env.sh"
scenario 14-entry-auth-membership

R="$(bare_remote team)"
new_vault alice team "$R"
put alice Alpha a1
V="$(vault_dir alice team)"

# last_commit_files <persona>: the paths touched by that clone's HEAD.
last_commit_files() { git -C "$(vault_dir "$1" team)" show --name-only --format= HEAD; }

# save_entry <persona> <title> <file>: copy that entry's file aside.
save_entry() {
    local id
    id="$(full_id "$1" team "$2")"
    cp "$(entry_file "$1" team "$id")" "$3"
    SAVED_ID="$id"
}

# restore_saved <persona> <file>: put a saved file back over its entry,
# uncommitted (a read sees the working tree).
restore_saved() { cp "$2" "$(entry_file "$1" team "$SAVED_ID")"; }

step "bob joins and writes"
join_by_key bob "$R" alice
g bob -i "$(unlock bob)"$'\nb1' -- --yes insert Bravo --value-stdin
expect_exit 0

step "adding carol re-seals the key file and re-encrypts, readable by her"
g alice -i "$(unlock alice)" -e 'QA_EDIT_EXPR=s/^value: .*/value: a2/' -- edit Alpha
join_by_key carol "$R" alice
expect_true "the add commit touched the key file" bash -c "git -C '$V' log -1 --name-only --format= -- .gage/vault-key.age | grep -q vault-key"
expect_true "the add is the latest commit that touches it" \
    test "$(git -C "$V" log -1 --format=%H -- .gage/vault-key.age)" = "$(head_of alice team)"
g carol -i "$(unlock carol)" -- show Alpha
expect_out '^a2$'
g carol -i "$(unlock carol)" -- show Bravo
expect_exit 0
expect_out '^b1$'

step "carol writes and every other recipient reads it"
g carol -i "$(unlock carol)"$'\nc1' -- --yes insert Charlie --value-stdin
expect_exit 0
for p in alice bob; do
    g "$p" -i "$(unlock "$p")" -- show Charlie
    expect_exit 0
    expect_out '^c1$'
done

step "an add does not rotate: an entry saved before it still verifies"
save_entry alice Bravo "$QA_S/bravo.before-add"
join_by_key dave "$R" alice
restore_saved alice "$QA_S/bravo.before-add"
g alice -i "$(unlock alice)" -- show Bravo
expect_exit 0
expect_out '^b1$'
git -C "$V" checkout -q -- entries

step "history: a revision from before the member joined is encrypted to the old list"
g dave -i "$(unlock dave)" -- history --decrypt Alpha
expect_exit 5
g carol -i "$(unlock carol)" -- history --decrypt Charlie
expect_exit 0
g dave -i "$(unlock dave)"$'\nd1' -- --yes insert Delta --value-stdin
expect_exit 0
g dave -i "$(unlock dave)" -- history --decrypt Delta
expect_exit 0
expect_out 'd1'

step "enrollment: approving erin re-seals the key file and she can read and write"
save_entry alice Bravo "$QA_S/bravo.before-enroll"
g erin -- clone "$R" --device erin-laptop
g erin -i "$(lines "$(pw erin)" "$(pw erin)")" -- identity enroll --device erin-laptop
expect_exit 0
ERIN_CODE="$(grep -oE 'GAGE(-[A-Z0-9]{4}){4}' <<<"$OUT$ERR" | head -1)"
ERIN_KEY="$(grep -oE 'age1[0-9a-z]{58}' <<<"$OUT$ERR" | head -1)"
g alice -- pull
g alice -i "$(lines y "$(pw alice)")" -- recipient approve --code "$ERIN_CODE"
expect_exit 0
expect_true "the approval touched the key file" \
    test "$(git -C "$V" log -1 --format=%H -- .gage/vault-key.age)" = "$(head_of alice team)"
g erin -i "$(unlock erin)" -- sync
expect_exit 0
for t in Alpha Bravo Charlie Delta; do
    g erin -i "$(unlock erin)" -- show "$t"
    expect_exit 0
done
g erin -i "$(unlock erin)"$'\ne1' -- --yes insert Echo --value-stdin
expect_exit 0
for p in alice bob carol dave; do
    g "$p" -i "$(unlock "$p")" -- show Echo
    expect_exit 0
    expect_out '^e1$'
done
restore_saved alice "$QA_S/bravo.before-enroll"
g alice -i "$(unlock alice)" -- show Bravo
expect_exit 0
git -C "$V" checkout -q -- entries

step "the first recovery key only grows the list: a saved entry still verifies"
save_entry alice Bravo "$QA_S/bravo.before-rk"
g alice -i "$(unlock alice)" -- recovery rotate --recovery-key-out "$QA_S/rk1"
expect_exit 0
g alice -- recipient list
expect_out '^recovery-paper-key '
restore_saved alice "$QA_S/bravo.before-rk"
g alice -i "$(unlock alice)" -- show Bravo
expect_exit 0
expect_out '^b1$'
git -C "$V" checkout -q -- entries

step "removing dave: entries, key file and recipient files in one commit"
for p in bob carol; do g "$p" -- pull; done
save_entry alice Bravo "$QA_S/bravo.before-remove"
# dave writes offline before he is removed; his clone hasn't seen the removal.
g dave -- git set-remote team "$QA_S/remotes/nowhere.git"
g dave -i "$(unlock dave)"$'\nevil' -- --yes insert Evil --value-stdin
expect_exit 0
EVIL_FILE="$(ls -t "$(vault_dir dave team)"/entries/*.age | head -1)"
EVIL_ID="$(basename "$EVIL_FILE" .age)"
cp "$EVIL_FILE" "$QA_S/evil.age"
g alice -i "$(unlock alice)" -- recipient remove dave-laptop --reencrypt
expect_exit 0
files="$(last_commit_files alice)"
expect_true "the commit has the key file" grep -q '^.gage/vault-key.age$' <<<"$files"
expect_true "the commit has the recipient files" grep -q '^.age-recipients$' <<<"$files"
expect_true "the commit has config.toml" grep -q '^.gage/config.toml$' <<<"$files"
expect_true "the commit has re-encrypted entries" grep -q '^entries/' <<<"$files"
expect_true "it is a single commit on top of the last write" \
    test "$(git -C "$V" rev-list --count HEAD~1..HEAD)" = 1

step "the remover and the others still read everything and read each other's writes"
for p in alice bob carol; do
    g "$p" -- pull
    for t in Alpha Bravo Charlie Echo; do
        g "$p" -i "$(unlock "$p")" -- show "$t"
        expect_exit 0
    done
done
put alice FromAlice fa
g bob -i "$(unlock bob)"$'\nfb' -- --yes insert FromBob --value-stdin
expect_exit 0
g alice -i "$(unlock alice)" -- show FromBob
expect_exit 0
expect_out '^fb$'
g bob -i "$(unlock bob)" -- show FromAlice
expect_exit 0
expect_out '^fa$'

step "a file saved before the removal is refused, though its holder can decrypt it"
restore_saved alice "$QA_S/bravo.before-remove"
g alice -i "$(unlock alice)" -- show Bravo
expect_exit 1
expect_err 'failed authentication'
expect_err "$SAVED_ID"
expect_no_out 'b1'
git -C "$V" checkout -q -- entries
g alice -i "$(unlock alice)" -- show Bravo
expect_exit 0

step "history still reads revisions written under the retired key"
# Bravo was written by bob, so every revision is one he can decrypt.
g bob -i "$(unlock bob)" -- history --decrypt Bravo
expect_exit 0
expect_out 'b1'

step "a removed member's write, pushed with plain git, is refused by name"
stranger_push "$R" "dave's last write" "put:$QA_S/evil.age:entries/$EVIL_ID.age"
for p in alice bob; do
    g "$p" -i "$(unlock "$p")" -- ls
    expect_exit 1
    expect_err 'failed authentication'
    expect_err "$EVIL_ID"
    expect_no_out 'evil'
done
g alice -- pull
git -C "$V" rm -q "entries/$EVIL_ID.age"
git -C "$V" -c user.name=alice -c user.email=a@example.invalid commit -q -m "remove dave's entry"
as alice -- git -C "$V" push -q origin HEAD:main
g bob -i "$(unlock bob)" -- ls
expect_exit 0
expect_no_out 'Evil'

step "a removed device can't finish a merge through gage: sync fails 5, pull and push 1"
g erin -- git set-remote team "$QA_S/remotes/nowhere.git"
g erin -i "$(unlock erin)"$'\noffline' -- --yes insert ErinOffline --value-stdin
expect_exit 0
g alice -i "$(unlock alice)" -- recipient remove erin-laptop --reencrypt
expect_exit 0
g erin -- git set-remote team "$R"
ERIN_HEAD="$(head_of erin team)"
REMOTE_HEAD="$(git -C "$R" rev-parse main)"
g erin -i "$(unlock erin)" -- sync
expect_exit 5
expect_true "erin's HEAD did not move" test "$(head_of erin team)" = "$ERIN_HEAD"
expect_true "nothing was pushed" test "$(git -C "$R" rev-parse main)" = "$REMOTE_HEAD"
g erin -- pull
expect_exit 1
g erin -- push
expect_exit 1

step "history after a second rotation"
g bob -- pull
g bob -i "$(unlock bob)" -- history --decrypt Bravo
expect_exit 0
expect_out 'b1'

step "remove, then re-add: erin returns and writes what the others accept"
as erin -- git -C "$(vault_dir erin team)" reset -q --hard "$ERIN_HEAD"
g alice -i "$(unlock alice)" -- recipient add "$ERIN_KEY" --device erin-laptop
expect_exit 0
g erin -i "$(unlock erin)" -- sync
expect_exit 0
g erin -i "$(unlock erin)" -- show FromBob
expect_exit 0
g erin -i "$(unlock erin)"$'\nback' -- --yes insert Back --value-stdin
expect_exit 0
for p in alice bob carol; do
    g "$p" -i "$(unlock "$p")" -- show Back
    expect_exit 0
    expect_out '^back$'
done

step "every write path after a rotation stamps a tag the others accept"
new_vault alice personal ""
put alice Wifi wifi-pass -u personal
put alice Bank bank-pass -u personal
put alice Plain p1
g alice -i "$(unlock alice)" -- generate Gen -l 12
expect_exit 0
g alice -i "$(unlock alice)" -e 'QA_EDIT_EXPR=s/^value: .*/value: p1-edited/' -- edit Plain
expect_exit 0
g alice -i "$(unlock alice)" -- rename Gen Gen2
expect_exit 0
TWO="$(unlock alice "$(pw alice)")"
g alice -i "$TWO" -- cp Wifi --to-vault team -u personal
expect_exit 0
g alice -i "$TWO" -- mv Bank --to-vault team -u personal
expect_exit 0
g bob -- pull
g bob -i "$(unlock bob)" -- ls
expect_exit 0
for t in Gen2 Wifi; do
    g bob -i "$(unlock bob)" -- show "$t"
    expect_exit 0
done
g bob -i "$(unlock bob)" -- show Bank
expect_out '^bank-pass$'
g bob -i "$(unlock bob)" -- show Plain
expect_out '^p1-edited$'

step "recovery rotate, replacing a key, rotates the MAC key"
save_entry alice Bravo "$QA_S/bravo.before-rotate"
g alice -i "$(unlock alice)" -- recovery rotate --recovery-key-out "$QA_S/rk2"
expect_exit 0
expect_true "the rotation touched the key file" \
    test "$(git -C "$V" log -1 --format=%H -- .gage/vault-key.age)" = "$(head_of alice team)"
restore_saved alice "$QA_S/bravo.before-rotate"
g alice -i "$(unlock alice)" -- show Bravo
expect_exit 1
expect_err 'failed authentication'
git -C "$V" checkout -q -- entries
g bob -- pull
g bob -i "$(unlock bob)" -- show Bravo
expect_exit 0

step "recovery enroll --replaces rotates it too; the recovering device reads and writes"
g bob -i "$(unlock bob)" -- ls
save_entry bob Bravo "$QA_S/bravo.before-enroll-replaces"
BV="$(vault_dir bob team)"
g alice2 -- clone "$R" --device alice-laptop
expect_exit 0
g alice2 -i "$(lines "$(cat "$QA_S/rk2")" "$(pw alice2)" "$(pw alice2)")" -- \
    recovery enroll --device alice-desktop --replaces alice-laptop --recovery-key-out "$QA_S/rk3"
expect_exit 0
for t in Alpha Bravo Charlie Plain; do
    g alice2 -i "$(unlock alice2)" -- show "$t"
    expect_exit 0
done
g bob -- pull
restore_saved bob "$QA_S/bravo.before-enroll-replaces"
g bob -i "$(unlock bob)" -- show Bravo
expect_exit 1
expect_err 'failed authentication'
git -C "$BV" checkout -q -- entries
g alice2 -i "$(unlock alice2)"$'\nr1' -- --yes insert Recovered --value-stdin
expect_exit 0
g bob -i "$(unlock bob)" -- show Recovered
expect_exit 0
expect_out '^r1$'

finish
