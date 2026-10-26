#!/usr/bin/env bash
# Sharing by enrollment: bob clones and publishes a sealed request; alice
# pulls, lists it, and approves it with the out-of-band code. Then carol is
# denied, and wrong codes, the approval prompt with nobody to answer it, and
# the request's opacity on the remote.
source "$(dirname "$0")/../qa-env.sh"
scenario 04-enrollment

R="$(bare_remote team)"
new_vault alice team "$R"
put alice Shared s3cret

step "bob enrolls: a key, a sealed request pushed, a code printed"
g bob -- clone "$R" --device bob-laptop
expect_exit 0
g bob -i "$(lines "$(pw bob)" "$(pw bob)")" -- identity enroll --device bob-laptop
expect_exit 0
BOB_CODE="$(grep -oE 'GAGE(-[A-Z0-9]{4}){4}' <<<"$OUT$ERR" | head -1)"
BOB_KEY="$(grep -oE 'age1[0-9a-z]{58}' <<<"$OUT$ERR" | head -1)"
expect_true "enroll printed a code" test -n "$BOB_CODE"
expect_true "the request is on the remote" bash -c "git -C '$R' ls-tree -r --name-only main | grep -q '^.gage/pending/'"
expect_true "the request names neither the device nor the key in clear" \
    bash -c "! git -C '$R' log -p main -- .gage/pending | grep -qE 'bob-laptop|$BOB_KEY'"

step "pending doesn't fetch; after a pull alice sees one request, no names"
g alice -- recipient pending
expect_exit 0
expect_out 'no pending'
g alice -- pull
expect_exit 0
g alice -- recipient pending
expect_exit 0
expect_out '1 pending enrollment request'
expect_no_out 'bob-laptop'

step "a wrong code opens nothing"
g alice -i "$(lines y "$(pw alice)")" -- recipient approve --code GAGE-AAAA-BBBB-CCCC-DDDD
expect_exit 5
g alice -i "$(lines y "$(pw alice)")" -- recipient approve --code not-a-code
expect_true "a malformed code is refused" test "$CODE" -ne 0

step "approve with nobody to answer is refused; --yes doesn't answer it"
g alice -i "" -- recipient approve --code "$BOB_CODE"
expect_exit 5
g alice -i "" -- --yes recipient approve --code "$BOB_CODE"
expect_exit 5
g alice -- recipient pending
expect_out '1 pending'

step "declining leaves the request pending"
g alice -i "$(lines n)" -- recipient approve --code "$BOB_CODE"
expect_exit 1
expect_out 'device: +bob-laptop'

step "approving shows who is asking, then re-encrypts and pushes"
g alice -i "$(lines y "$(pw alice)")" -- recipient approve --code "$BOB_CODE"
expect_exit 0
expect_out "pubkey: +$BOB_KEY"
expect_out 'approved "bob-laptop"'
expect_out '1 recipient added, 1 entry re-encrypted, 1 request cleared'
g alice -- recipient pending
expect_out 'no pending'
g alice -- recipient list
expect_out "^bob-laptop +$BOB_KEY"

step "the same code can't be used twice"
g alice -i "$(lines y "$(pw alice)")" -- recipient approve --code "$BOB_CODE"
expect_exit 5

step "bob syncs and reads"
g bob -i "$(unlock bob)" -- sync
expect_exit 0
g bob -i "$(unlock bob)" -- show Shared
expect_exit 0
expect_out '^s3cret$'

step "re-running enroll on an enrolled device reuses the key"
g bob -i "$(unlock bob)" -- identity enroll --device bob-laptop
expect_exit 0
expect_out 'already a recipient'
expect_out 'nothing was published'

step "carol enrolls and is denied"
g carol -- clone "$R" --device carol-laptop
g carol -i "$(lines "$(pw carol)" "$(pw carol)")" -- identity enroll --device carol-laptop --ttl 1h
expect_exit 0
g alice -- pull
g alice -- recipient pending
expect_out '1 pending'
REQ="$(grep -oE '^ +[0-9a-f]{8} ' <<<"$OUT" | tr -d ' ' | head -1)"
expect_true "pending listed a request id" test -n "$REQ"
g alice -- recipient deny "$REQ"
expect_exit 0
g alice -- recipient pending
expect_out 'no pending'
g carol -i "$(unlock carol)" -- sync
g carol -i "$(unlock carol)" -- show Shared
expect_exit 5
g alice -- recipient deny ffffffff
expect_exit 3

step "enroll's flags are validated"
g carol -i "$(unlock carol)" -- identity enroll --device carol-laptop --ttl 200h
expect_exit 2
g carol -i "$(unlock carol)" -- identity enroll --device '../evil'
expect_exit 2

step "enroll with no remote is a usage error"
new_vault dave solo ""
g dave -i "$(unlock dave)" -- identity enroll --device dave-2 -u solo
expect_exit 2

finish
