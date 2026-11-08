#!/usr/bin/env bash
# Merges that straddle a key change (ADR-0066). A device that wrote while
# offline merges back an entry tagged under a key that has since changed, and
# encrypted to a list that has since changed. The automatic merge stops, and
# `gage sync` re-seals the entry under the merged key and list. Also: a
# removed device can't complete that merge, a remote key that doesn't
# continue this one is refused, and a half-applied rotation refuses reads
# until the next write repairs it. Offline is a remote pointed at a path
# that doesn't exist, as in 06-sync.
source "$(dirname "$0")/../qa-env.sh"
scenario 15-entry-auth-sync

R="$(bare_remote team)"
NOWHERE="$QA_S/remotes/does-not-exist.git"
new_vault alice team "$R" --recovery-key-out "$QA_S/rk0"
put alice Shared base
V="$(vault_dir alice team)"
BV=
join_by_key bob "$R" alice
join_by_key carol "$R" alice
join_by_key dave "$R" alice
BV="$(vault_dir bob team)"
for p in bob carol dave; do g "$p" -i "$(unlock "$p")" -- ls; done

RK_N=0
# rotate_recovery <persona>: replace the recovery key, which drops one and so
# rotates the MAC key without costing the vault a member.
rotate_recovery() {
    RK_N=$((RK_N + 1))
    g "$1" -i "$(unlock "$1")" -- recovery rotate --recovery-key-out "$QA_S/rk-r$RK_N"
    expect_exit 0
}

step "offline writer across an add: sync, and the new recipient reads the entry"
g bob -- git set-remote team "$NOWHERE"
g bob -i "$(unlock bob)"$'\nb1' -- --yes insert FromBob --value-stdin
expect_exit 0
join_by_key erin "$R" alice
g bob -- git set-remote team "$R"
g bob -i "$(unlock bob)" -- sync
expect_exit 0
g erin -i "$(unlock erin)" -- sync
g erin -i "$(unlock erin)" -- show FromBob
expect_exit 0
expect_out '^b1$'
for p in alice carol; do
    g "$p" -i "$(unlock "$p")" -- show FromBob
    expect_exit 0
done

step "the rotator was offline: its sync re-stamps a writer's entry"
g alice -- git set-remote team "$NOWHERE"
rotate_recovery alice
g bob -i "$(unlock bob)"$'\nb2' -- --yes insert WhileAliceOffline --value-stdin
expect_exit 0
g alice -- git set-remote team "$R"
g alice -i "$(unlock alice)" -- sync
expect_exit 0
g alice -i "$(unlock alice)" -- show WhileAliceOffline
expect_exit 0
expect_out '^b2$'
for p in bob carol; do
    g "$p" -i "$(unlock "$p")" -- sync
    g "$p" -i "$(unlock "$p")" -- show WhileAliceOffline
    expect_exit 0
done

step "offline writer across a removal: pull and push refuse, sync merges and re-stamps"
g bob -- git set-remote team "$NOWHERE"
g bob -i "$(unlock bob)"$'\nb3' -- --yes insert OfflineAcrossRemoval --value-stdin
expect_exit 0
g alice -i "$(unlock alice)" -- recipient remove dave-laptop --reencrypt
expect_exit 0
g bob -- git set-remote team "$R"
BOB_HEAD="$(head_of bob team)"
g bob -- pull
expect_exit 1
expect_true "pull did not merge" test "$(head_of bob team)" = "$BOB_HEAD"
g bob -- push
expect_exit 1
expect_true "push did not merge" test "$(head_of bob team)" = "$BOB_HEAD"
g bob -i "$(unlock bob)"$'\nb4' -- --yes insert FurtherWrite --value-stdin
expect_exit 0
expect_err 'committed locally but not pushed|diverged'
g bob -i "$(unlock bob)" -- sync
expect_exit 0
for p in alice carol erin; do
    g "$p" -i "$(unlock "$p")" -- sync
    for t in OfflineAcrossRemoval FurtherWrite; do
        g "$p" -i "$(unlock "$p")" -- show "$t"
        expect_exit 0
    done
done
g dave -- pull
g dave -i "$(unlock dave)" -- show OfflineAcrossRemoval
expect_exit 5
expect_no_out 'b3'

# conflict_round <label>: bob edits Shared offline while alice edits it and
# rotates the MAC key, so the same entry changed on both sides across a key
# change. Leaves bob's remote reachable again and the divergence unresolved.
conflict_round() {
    g bob -- git set-remote team "$NOWHERE"
    g bob -i "$(unlock bob)" -e "QA_EDIT_EXPR=s/^value: .*/value: bob-$1/" -- --yes edit Shared
    expect_exit 0
    g alice -i "$(unlock alice)" -e "QA_EDIT_EXPR=s/^value: .*/value: alice-$1/" -- edit Shared
    expect_exit 0
    rotate_recovery alice
    g bob -- git set-remote team "$R"
}

# everyone_reads: each remaining recipient lists the vault after catching up.
everyone_reads() {
    local p
    for p in alice bob carol erin; do
        g "$p" -i "$(unlock "$p")" -- sync
        g "$p" -i "$(unlock "$p")" -- ls
        expect_exit 0
    done
}

step "a real conflict across a rotation: with nobody to ask, sync shows no value"
conflict_round one
g bob -i "$(unlock bob)" -- sync
expect_exit 1
expect_no_out 'alice-one|bob-one'
expect_no_err 'alice-one|bob-one'

for choice in l r b; do
    step "...and on a terminal, '$choice' leaves a vault everyone can read"
    [[ "$choice" != l ]] && conflict_round "$choice"
    gtty bob "$(dialogue "$PROMPT_PW" "$(pw bob)" 'Which\? \[l/r/b/s/q\]' "$choice")" -- sync
    expect_exit 0
    everyone_reads
    if [[ "$choice" == b ]]; then
        for p in alice bob carol erin; do
            g "$p" -i "$(unlock "$p")" -- search Shared
            expect_true "$p sees both versions" test "$(grep -c Shared <<<"$OUT")" -ge 2
        done
    else
        for p in alice bob carol erin; do
            g "$p" -i "$(unlock "$p")" -- show Shared
            expect_exit 0
        done
    fi
done

step "a remote key that doesn't continue this one is refused"
# mallory seals a key of her own to the team's public keys and pushes it over
# the real one. bob has a local write, so sync has a merge to make.
g bob -i "$(unlock bob)" -- sync
g bob -- git set-remote team "$NOWHERE"
g bob -i "$(unlock bob)"$'\nb5' -- --yes insert BeforeReplace --value-stdin
expect_exit 0
forge_entry "$R" Junk junk
stranger_push "$R" "replace the key" "put:$FORGED_KEY:.gage/vault-key.age"
g bob -- git set-remote team "$R"
BOB_HEAD="$(head_of bob team)"
REMOTE_HEAD="$(git -C "$R" rev-parse main)"
g bob -i "$(unlock bob)" -- sync
expect_exit 1
expect_err 'does not continue'
expect_true "bob's HEAD did not move" test "$(head_of bob team)" = "$BOB_HEAD"
expect_true "nothing was pushed" test "$(git -C "$R" rev-parse main)" = "$REMOTE_HEAD"

step "half-applied rotation: reads refuse, the next write discards it"
# A rotation that died after writing the entries and before the key file
# leaves post-rotation entries beside the old key file. Simulated here; the
# crash itself is covered in-process.
new_vault alice half "" --recovery-key-out "$QA_S/rk-half"
put alice Solo s1 -u half
HV="$(vault_dir alice half)"
cp "$HV/.gage/vault-key.age" "$QA_S/key.before"
g alice -i "$(unlock alice)" -- recovery rotate --recovery-key-out "$QA_S/rk-half2" -u half
expect_exit 0
cp "$QA_S/key.before" "$HV/.gage/vault-key.age"
g alice -i "$(unlock alice)" -- ls -u half
expect_exit 1
expect_err 'failed authentication'
expect_no_out 'Solo'
g alice -i "$(unlock alice)"$'\nh' -- --yes insert AfterHalf --value-stdin -u half
expect_exit 0
expect_err 'discarded uncommitted changes'
g alice -i "$(unlock alice)" -- ls -u half
expect_exit 0
expect_out '^Solo '
expect_out '^AfterHalf '

finish
