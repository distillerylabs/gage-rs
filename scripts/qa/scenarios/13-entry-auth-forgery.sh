#!/usr/bin/env bash
# Entry authentication through the real binary: what a stranger with push
# access can and can't do (ADR-0065, doc/cli/entries.md "When an entry fails
# to verify"). A forged entry is refused by every command that reads, and
# one bad entry stops the vault until it is removed with git. Also the key
# file's states, format v3 written and v2 refused, and the reserved `auth`
# key. mallory forges by running a vault of her own with the team's public
# keys as recipients (see forge_entry in qa-env.sh).
source "$(dirname "$0")/../qa-env.sh"
scenario 13-entry-auth-forgery

R="$(bare_remote team)"
new_vault alice team "$R"
put alice Real real-value
put alice Other other-value
V="$(vault_dir alice team)"

step "a new vault is format 3 with a tracked, unmergeable key file"
expect_true "format_version = 3" grep -q '^format_version = 3$' "$V/.gage/config.toml"
expect_true ".gage/vault-key.age is tracked" git -C "$V" ls-files --error-unmatch .gage/vault-key.age
expect_true ".gitattributes marks it -merge" grep -qxF '.gage/vault-key.age -merge' "$V/.gitattributes"

step "a stranger's forged entry fails every read, naming the id"
forge_entry "$R" Forged forged-value
stranger_push "$R" "add" "put:$FORGED_FILE:entries/$FORGED_ID.age"
for cmd in "show Real" "ls" "search Real"; do
    g alice -i "$(unlock alice)" -- $cmd
    expect_exit 1
    expect_err 'failed authentication'
    expect_err "$FORGED_ID"
    expect_no_out 'forged-value|real-value'
    expect_no_err 'forged-value|real-value'
done
g alice -e "GAGE_PASSPHRASE=$(pw alice)" -i "$(lines 'ls')" -- --stdin
expect_exit 1
expect_err 'failed authentication'
expect_err "$FORGED_ID"
expect_no_out 'forged-value'

step "with a bad entry present, a write fails too"
g alice -i "$(unlock alice)"$'\nnew' -- insert Fresh --value-stdin
expect_exit 1
expect_err 'failed authentication'

step "the documented way out: git rm the named file"
git -C "$V" rm -q "entries/$FORGED_ID.age"
git -C "$V" -c user.name=alice -c user.email=a@example.invalid commit -q -m "remove forged"
as alice -- git -C "$V" push -q origin HEAD:main
expect_exit 0
g alice -i "$(unlock alice)" -- ls
expect_exit 0
expect_out '^Real '
expect_out '^Other '
expect_no_out 'Forged'
g alice -i "$(unlock alice)" -- show Real
expect_out '^real-value$'

step "entries are bound to their id: a swap and a copy both fail"
REAL_ID="$(full_id alice team Real)"
OTHER_ID="$(full_id alice team Other)"
A="$(entry_file alice team "$REAL_ID")"
B="$(entry_file alice team "$OTHER_ID")"
cp "$A" "$QA_S/real.age" && cp "$B" "$QA_S/other.age"
cp "$QA_S/other.age" "$A" && cp "$QA_S/real.age" "$B"
g alice -i "$(unlock alice)" -- ls
expect_exit 1
expect_err 'failed authentication'
git -C "$V" checkout -q -- entries
g alice -i "$(unlock alice)" -- ls
expect_exit 0
COPY_ID="11111111-2222-4333-8444-555555555555"
cp "$A" "$(entry_file alice team "$COPY_ID")"
g alice -i "$(unlock alice)" -- ls
expect_exit 1
expect_err 'failed authentication'
expect_err "$COPY_ID"
rm -f "$(entry_file alice team "$COPY_ID")"

step "key file deleted but not committed: reads refuse, the next write repairs"
K="$V/.gage/vault-key.age"
cp "$K" "$QA_S/vault-key.good"
rm "$K"
g alice -i "$(unlock alice)" -- show Real
expect_exit 1
expect_err 'is missing'
expect_no_out 'real-value'
g alice -i "$(unlock alice)"$'\nw1' -- insert Repair --value-stdin
expect_exit 0
expect_true "the write discarded the deletion" test -f "$K"
g alice -i "$(unlock alice)" -- show Real
expect_exit 0
expect_out '^real-value$'

step "key file deleted and committed: reads and writes both refuse"
git -C "$V" rm -q .gage/vault-key.age
git -C "$V" -c user.name=alice -c user.email=a@example.invalid commit -q -m "drop key"
g alice -i "$(unlock alice)" -- show Real
expect_exit 1
expect_err 'is missing'
g alice -i "$(unlock alice)"$'\nw2' -- insert Blocked --value-stdin
expect_exit 1
git -C "$V" reset -q --hard HEAD~1

step "key file garbage: reads refuse as corrupt"
printf 'not an age file' >"$K"
g alice -i "$(unlock alice)" -- show Real
expect_exit 1
expect_err 'corrupt'
expect_no_out 'real-value'
git -C "$V" checkout -q -- .gage/vault-key.age

step "key file replaced, entries genuine: unlock warns and every read fails"
R2="$(bare_remote keyrep)"
new_vault alice keyrep "$R2"
put alice Pw genuine -u keyrep
forge_entry "$R2" Pw forged
stranger_push "$R2" "swap key" "put:$FORGED_KEY:.gage/vault-key.age"
g alice -i "$(unlock alice)" -- show Pw -u keyrep
expect_exit 1
expect_err 'was replaced on its own'
expect_no_out 'genuine|forged'

step "a write after a replaced key refuses without a terminal and writes nothing"
KR="$(vault_dir alice keyrep)"
BEFORE="$(head_of alice keyrep)"
g alice -i "$(unlock alice)"$'\nx' -- insert Nope --value-stdin -u keyrep
expect_exit 1
expect_err 'nobody is at a terminal'
expect_true "nothing was committed" test "$(head_of alice keyrep)" = "$BEFORE"

step "on a terminal the review names the key file; declining writes nothing"
gtty alice "$(dialogue "$PROMPT_PW" "$(pw alice)" 'Value for' x 'trust this recipient list\?' n)" -- insert Nope -u keyrep
expect_exit 1
expect_out 'vault-key.age was replaced'
expect_true "nothing was committed" test "$(head_of alice keyrep)" = "$BEFORE"

step "vault replaced wholesale: every entry and the key file"
# Pinned today, and a known gap: someone with push access replaces the key
# file and every entry, and gage prints the forged value and exits 0, with
# one line on stderr. The warning stops after one --yes write. #207 fixes
# this and flips these assertions; do not loosen them until it does.
R3="$(bare_remote swap)"
new_vault alice swap "$R3"
put alice Pw genuine -u swap
SW="$(vault_dir alice swap)"
GENUINE_ID="$(full_id alice swap Pw)"
forge_entry "$R3" Pw forged
stranger_push "$R3" "replace everything" "rm:entries/$GENUINE_ID.age" \
    "put:$FORGED_FILE:entries/$FORGED_ID.age" "put:$FORGED_KEY:.gage/vault-key.age"
g alice -i "$(unlock alice)" -- show Pw -u swap
expect_exit 0
expect_out '^forged$'
expect_err 'was replaced on its own'
g alice -i "$(unlock alice)" -- ls -u swap
expect_exit 0
expect_out '^Pw '
g alice -e "GAGE_PASSPHRASE=$(pw alice)" -i "$(lines 'show --use swap Pw')" -- --stdin
expect_exit 0
expect_out '^forged$'
g alice -i "$(unlock alice)"$'\nw' -- --yes insert After --value-stdin -u swap
expect_exit 0
g alice -i "$(unlock alice)" -- show Pw -u swap
expect_exit 0
expect_out '^forged$'
expect_no_err 'was replaced on its own'

step "auth is a reserved top-level key; as a --field it is just a field"
# A hand-written auth: line would be taken for the entry's tag.
AUTH_EDIT=$'s/^value: .*/&\\\nauth: x/'
g alice -i "$(unlock alice)" -e "QA_EDIT_EXPR=$AUTH_EDIT" -- edit Real
expect_exit 2
g alice -i "$(unlock alice)" -- show Real
expect_exit 0
expect_out '^real-value$'
g alice -i "$(unlock alice)"$'\nfv' -- insert WithAuthField --value-stdin --field auth=x
expect_exit 0
g alice -i "$(unlock alice)" -- show WithAuthField --field auth
expect_exit 0
expect_out '^x$'

step "a v2 vault is refused on clone, leaving nothing behind"
V2="$QA_S/remotes/v2.git"
cp -R "$QA_REPO/tests/fixtures/format-v2/vault.git" "$V2"
g carol -- clone "$V2" --device carol-laptop
expect_exit 1
expect_err 'older format'
g carol -- vault list
expect_no_out 'v2'
expect_true "no vault directory was left" bash -c "! ls '$(persona_home carol)'/data/gage/vaults/* >/dev/null 2>&1"

step "a vault downgraded to format_version = 2 is refused"
sed 's/^format_version = 3$/format_version = 2/' "$V/.gage/config.toml" >"$QA_S/config.v2" &&
    cp "$QA_S/config.v2" "$V/.gage/config.toml"
g alice -i "$(unlock alice)" -- show Real
expect_exit 1
expect_no_out 'real-value'
git -C "$V" checkout -q -- .gage/config.toml

step "the format-v3 fixture clones"
V3="$QA_S/remotes/v3.git"
cp -R "$QA_REPO/tests/fixtures/format-v3/vault.git" "$V3"
g dave -- clone "$V3" --device dave-laptop
expect_exit 0

finish
