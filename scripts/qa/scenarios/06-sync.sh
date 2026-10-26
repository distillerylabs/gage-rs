#!/usr/bin/env bash
# Two writers, one remote. Every unlock fetches and fast-forwards and every
# write pushes (doc/cli/sync.md), so divergence needs a writer who was
# offline: bob's origin is pointed at a path that doesn't exist while both
# write. Covers auto-push, an unreachable remote (exit 7 for sync commands,
# a warning for writes), push/pull refusing diverged history, sync merging
# different entries, a real conflict kicked to a human (refused without one,
# resolved on a terminal), log, and git set-remote.
source "$(dirname "$0")/../qa-env.sh"
scenario 06-sync

R="$(bare_remote team)"
NOWHERE="$QA_S/remotes/does-not-exist.git"
new_vault alice team "$R"
put alice Shared base
g bob -- clone "$R" --device bob-laptop
g bob -i "$(lines "$(pw bob)" "$(pw bob)")" -- identity add --device bob-laptop
BOB_KEY="$(grep -oE 'age1[0-9a-z]{58}' <<<"$OUT$ERR" | head -1)"
g alice -i "$(unlock alice)" -- recipient add "$BOB_KEY" --device bob-laptop
g bob -i "$(unlock bob)"$'\nx' -- --yes insert Confirm --value-stdin
expect_exit 0

step "every write pushes, every unlock catches up"
g alice -i "$(unlock alice)" -- show Confirm
expect_out '^x$'
expect_true "bob's insert is on the remote" test "$(git -C "$R" rev-parse main)" = "$(git -C "$(vault_dir bob team)" rev-parse main)"

step "offline, sync commands fail 7 and writes warn and carry on"
g bob -- git set-remote team "$NOWHERE"
expect_exit 0
g bob -- pull
expect_exit 7
g bob -- push
expect_exit 7
g bob -i "$(unlock bob)" -- sync
expect_exit 7
g bob -i "$(unlock bob)"$'\nb1' -- insert FromBob --value-stdin
expect_exit 0
expect_err 'committed locally but not pushed'
g bob -i "$(unlock bob)" -- show FromBob
expect_exit 0
expect_out '^b1$'

step "both write while bob is offline: histories diverge"
put alice FromAlice a1
g bob -- git set-remote team "$R"
expect_exit 0
BOB_HEAD="$(git -C "$(vault_dir bob team)" rev-parse HEAD)"

step "pull reports the divergence and never merges"
g bob -- pull
expect_exit 1
expect_true "pull left bob's history alone" test "$(git -C "$(vault_dir bob team)" rev-parse HEAD)" = "$BOB_HEAD"

step "push merges a divergence over different entries, then pushes"
g bob -- push
expect_exit 0
expect_out 'merged'
g alice -i "$(unlock alice)" -- show FromBob
expect_out '^b1$'
g bob -i "$(unlock bob)" -- show FromAlice
expect_out '^a1$'

step "an unlock while diverged warns; the write's push then merges"
g bob -- git set-remote team "$NOWHERE"
g bob -i "$(unlock bob)"$'\nb2' -- insert FromBob2 --value-stdin
put alice FromAlice2 a2
g bob -- git set-remote team "$R"
g bob -i "$(unlock bob)"$'\nb3' -- insert FromBob3 --value-stdin
expect_exit 0
expect_err 'diverged'
expect_true "bob's write reached the remote" \
    test "$(git -C "$R" rev-parse main)" = "$(git -C "$(vault_dir bob team)" rev-parse HEAD)"

step "sync merges different entries and pushes"
g bob -- git set-remote team "$NOWHERE"
g bob -i "$(unlock bob)"$'\nb4' -- insert FromBob4 --value-stdin
put alice FromAlice3 a3
g bob -- git set-remote team "$R"
g bob -- sync
expect_exit 0
g alice -i "$(unlock alice)" -- show FromBob4
expect_out '^b4$'
g bob -i "$(unlock bob)" -- show FromAlice3
expect_out '^a3$'

step "both edit the same entry while bob is offline: a real conflict"
g bob -- git set-remote team "$NOWHERE"
g alice -i "$(unlock alice)" -e 'QA_EDIT_EXPR=s/^value: .*/value: alice-edit/' -- edit Shared
expect_exit 0
g bob -i "$(unlock bob)" -e 'QA_EDIT_EXPR=s/^value: .*/value: bob-edit/' -- edit Shared
expect_exit 0
g bob -- git set-remote team "$R"

step "sync with nobody to ask stops without choosing or showing values"
g bob -i "$(unlock bob)" -- sync
expect_exit 1
expect_no_out 'alice-edit|bob-edit'
expect_no_err 'alice-edit|bob-edit'
g bob -i "$(unlock bob)" -- show Shared
expect_out '^bob-edit$'

step "on a terminal the menu shows provenance, never the values"
gtty bob "$(dialogue "$PROMPT_PW" "$(pw bob)" 'Which\? \[l/r/b/s/q\]' q)" -- sync
expect_exit 1
expect_out '\[l/r/b/s/q\]'
expect_out 'alice-laptop'
expect_no_out 'alice-edit|bob-edit'
g bob -i "$(unlock bob)" -- show Shared
expect_out '^bob-edit$'

step "resolving with r keeps the remote's version and pushes"
gtty bob "$(dialogue "$PROMPT_PW" "$(pw bob)" 'Which\? \[l/r/b/s/q\]' r)" -- sync
expect_exit 0
g bob -i "$(unlock bob)" -- show Shared
expect_out '^alice-edit$'
g alice -i "$(unlock alice)" -- show Shared
expect_out '^alice-edit$'

step "resolving with b keeps both"
g bob -- git set-remote team "$NOWHERE"
g alice -i "$(unlock alice)" -e 'QA_EDIT_EXPR=s/^value: .*/value: alice-2/' -- edit Shared
g bob -i "$(unlock bob)" -e 'QA_EDIT_EXPR=s/^value: .*/value: bob-2/' -- edit Shared
g bob -- git set-remote team "$R"
gtty bob "$(dialogue "$PROMPT_PW" "$(pw bob)" 'Which\? \[l/r/b/s/q\]' b)" -- sync
expect_exit 0
g alice -i "$(unlock alice)" -- search Shared
expect_true "keep-both leaves two entries" test "$(grep -c Shared <<<"$OUT")" -ge 2

step "log shows commits by timestamp without decrypting"
# log still unlocks: resolving titles means decrypting (doc/cli/entries.md).
g alice -- log
expect_exit 5
g alice -i "$(unlock alice)" -- log
expect_exit 0
expect_out '^[0-9]{4}-[0-9]{2}-[0-9]{2} .* [0-9a-f]{8}$'
expect_no_out 'alice-edit|bob-edit|base'
g alice -i "$(unlock alice)" -- log FromAlice
expect_exit 0
expect_out '^[0-9]{4}-[0-9]{2}-[0-9]{2} '

step "git set-remote moves the origin; vault info shows it"
R2="$(bare_remote team-mirror)"
git -C "$R" push -q --mirror "$R2"
g alice -- git set-remote team "$R2"
expect_exit 0
g alice -- vault info team
expect_out "remote: .*team-mirror"
put alice AfterMove m1
expect_true "the write landed on the new remote only" \
    test "$(git -C "$R2" rev-parse main)" != "$(git -C "$R" rev-parse main)"
g alice -- git set-remote nope "$R2"
expect_exit 3

finish
