#!/usr/bin/env bash
# The HTTPS token store, without contacting any host: login reads the token
# from the prompt (never an argument), status names hosts and never prints a
# token, the file is 0600 in a 0700 directory, logout forgets it. Nothing
# here talks to the network: no command below needs to reach a remote.
source "$(dirname "$0")/../qa-env.sh"
scenario 12-auth

TOKEN="ghp_qaFakeToken0123456789abcdefghijklmnop"
TOK_DIR="$(persona_home alice)/state/gage/tokens"

step "auth status with nothing stored"
g alice -- auth status --host git.example.test
expect_exit 0
expect_no_out "$TOKEN"

step "auth login stores a token read from the prompt"
g alice -i "$(lines "$TOKEN")" -- auth login --host git.example.test
expect_exit 0
expect_no_out "$TOKEN"
expect_no_err "$TOKEN"
expect_true "token file exists" test -f "$TOK_DIR/git.example.test"
expect_true "token file is mode 0600" test "$(file_mode "$TOK_DIR/git.example.test")" = 600
expect_true "tokens dir is mode 0700" test "$(file_mode "$TOK_DIR")" = 700

step "auth status names the host, never the token"
g alice -- auth status
expect_exit 0
expect_out 'git.example.test'
expect_no_out "$TOKEN"
g alice -- auth status --host git.example.test
expect_exit 0
expect_no_out "$TOKEN"

step "the token is never an argument"
g alice -- auth login --host git.example.test "$TOKEN"
expect_exit 2
g alice -- auth login --token "$TOKEN" --host git.example.test
expect_exit 2

step "an empty token is not stored"
g alice -i "" -- auth login --host empty.example.test
expect_true "empty token is refused or skipped" test ! -s "$TOK_DIR/empty.example.test"

step "a token file others can read is reported, not used"
chmod 644 "$TOK_DIR/git.example.test"
g alice -- auth status --host git.example.test
expect_no_out "$TOKEN"
expect_no_err "$TOKEN"
chmod 600 "$TOK_DIR/git.example.test"

step "auth logout forgets it"
g alice -- auth logout --host git.example.test
expect_exit 0
expect_true "token file removed" test ! -e "$TOK_DIR/git.example.test"
g alice -- auth logout --host git.example.test
expect_true "logging out twice is harmless" test "$CODE" -le 3

step "with no --host and no vault, there is no default host"
g alice -- auth status
expect_exit 0
g alice -i "$(lines "$TOKEN")" -- auth login
expect_exit 2

step "the token store is never inside a vault or the config dir"
new_vault alice v ""
g alice -i "$(lines "$TOKEN")" -- auth login --host git.example.test
expect_true "no token under config" bash -c "! grep -rqs '$TOKEN' '$(persona_home alice)/config'"
expect_true "no token under data" bash -c "! grep -rqs '$TOKEN' '$(persona_home alice)/data'"

finish
