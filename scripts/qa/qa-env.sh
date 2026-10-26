# shellcheck shell=bash
# The QA harness: personas, a gage runner that records what it ran, and
# assertions. Sourced by every scenario under scenarios/, and by hand:
#
#   source scripts/qa/qa-env.sh
#   scenario scratch
#   g alice -i "$(pw alice)"$'\n'"$(pw alice)"$'\n' -- init team --device alice-laptop --no-recovery-key
#   expect_exit 0
#
# A persona is a person on their own machine: its own HOME and XDG roots,
# so its config, identities, trust cache and locks are its own. Personas
# share vaults only through bare git repositories under $QA_S/remotes,
# which are local paths: no token, no network.
#
# Nothing here reads or writes the real ~/.config/gage, ~/.local/share/gage
# or ~/.local/state/gage. Every gage process gets the persona's roots.

QA_REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
QA_DIR="$QA_REPO/scripts/qa"
GAGE="${GAGE:-$QA_REPO/target/debug/gage}"

# The run's root. run-all.sh sets it once for every scenario; a scenario run
# on its own gets a fresh one.
if [[ -z "${QA:-}" ]]; then
    QA="$(mktemp -d "${TMPDIR:-/tmp}/gage-qa.XXXXXX")"
fi
export QA
mkdir -p "$QA"

QA_RESULTS="$QA/results.tsv"
QA_COVERAGE="$QA/coverage.tsv"

# The scenario's state: its root ($QA_S), name, current step, counters.
QA_SCENARIO=""
QA_S=""
QA_STEP=""
QA_N=0
QA_FAILS=0
QA_PASSES=0

# What the last `g`/`as` produced.
OUT=""
ERR=""
CODE=""

if [[ ! -x "$GAGE" ]]; then
    echo "qa: no gage binary at $GAGE (run 'make build', or set GAGE)" >&2
    return 1 2>/dev/null || exit 1
fi

# scenario <name>: start a scenario with its own personas and remotes, so a
# scenario never depends on another having run.
scenario() {
    QA_SCENARIO="$1"
    QA_S="$QA/s/$1"
    rm -rf "$QA_S"
    mkdir -p "$QA_S/remotes" "$QA_S/log" "$QA_S/out"
    QA_N=0 QA_FAILS=0 QA_PASSES=0 QA_STEP=""
    : >"$QA_S/log/transcript.txt"
    echo "== scenario $1 ($QA_S)"
}

# step <description>: name what the next commands and assertions prove.
step() {
    QA_STEP="$*"
    printf '\n## %s\n' "$QA_STEP" >>"$QA_S/log/transcript.txt"
}

# pw <persona>: the persona's passphrase. Fixed per persona so a transcript
# can be replayed, and redacted from the logs anyway.
pw() { printf 'pw-%s-correct-horse' "$1"; }

# lines <s>...: each argument followed by a newline, for stdin.
lines() { printf '%s\n' "$@"; }

# persona_home <persona>: the root of that persona's machine.
persona_home() { printf '%s/%s' "$QA_S" "$1"; }

# _persona_env <persona>: the environment of that persona's machine. PATH
# and TMPDIR pass through; nothing else from the caller does, so a real
# EDITOR, GAGE_PASSPHRASE or XDG_* can't leak in.
_persona_env() {
    local h
    h="$(persona_home "$1")"
    mkdir -p "$h/home" "$h/config" "$h/data" "$h/state"
    printf '%s\n' \
        "HOME=$h/home" \
        "XDG_CONFIG_HOME=$h/config" \
        "XDG_DATA_HOME=$h/data" \
        "XDG_STATE_HOME=$h/state" \
        "PATH=$PATH" \
        "TMPDIR=${TMPDIR:-/tmp}" \
        "LANG=C.UTF-8" \
        "TERM=dumb" \
        "EDITOR=$QA_DIR/editor-sed.sh"
}

# _redact <text>: the text with every persona passphrase masked.
_redact() {
    printf '%s' "$1" | sed -E 's/pw-([A-Za-z0-9]+)-correct-horse/<pw:\1>/g'
}

# _command_name <args...>: which registered command the args name, for the
# coverage table. Two-word groups first, then the bare word.
_command_name() {
    local a b skip=0 words=()
    for a in "$@"; do
        if ((skip)); then skip=0; continue; fi
        case "$a" in
        --script|-u|--use) skip=1; words+=("$a"); continue ;;
        --stdin|--yes) words+=("$a"); continue ;;
        -*) continue ;;
        esac
        words+=("$a")
        ((${#words[@]} >= 3)) && break
    done
    a="${words[0]:-}" b="${words[1]:-}"
    case "$a" in
    --script|--stdin) printf 'session(%s)' "$a"; return ;;
    --yes) _command_name "${@:2}"; return ;;
    vault|identity|recipient|recovery|auth) printf '%s %s' "$a" "$b" ;;
    git) printf 'git %s' "$b" ;;
    help) printf 'help' ;;
    "") printf 'session' ;;
    *) printf '%s' "$a" ;;
    esac
}

# g <persona> [-i STDIN] [-e K=V]... [--] <gage args...>
#
# Runs one gage command as <persona>, with STDIN (default: empty, never the
# caller's terminal) and any extra environment. Sets OUT, ERR and CODE, and
# appends the full exchange to the scenario's transcript.
g() {
    local who="$1" input="" extra=()
    shift
    while (($#)); do
        case "$1" in
        -i) input="$2"; shift 2 ;;
        -e) extra+=("$2"); shift 2 ;;
        --) shift; break ;;
        *) break ;;
        esac
    done
    _run "$who" "$input" "${extra[@]+"${extra[@]}"}" -- "$GAGE" "$@"
    printf '%s\t%s\t%s\n' "$QA_SCENARIO" "$(_command_name "$@")" "$CODE" >>"$QA_COVERAGE"
}

# gtty <persona> <dialogue> [-e K=V]... [--] <gage args...>
#
# Runs one gage command on a real terminal through tty.exp, answering the
# prompts in <dialogue> ("<regex>\t<reply>" per line; see dialogue below).
# For the paths that refuse piped stdin by design. OUT is everything the
# terminal showed (stdout and stderr together). Without expect installed,
# CODE is "skip" and the expect_* assertions record SKIP, not FAIL.
gtty() {
    local who="$1" dlg="$2" extra=()
    shift 2
    while (($#)); do
        case "$1" in
        -e) extra+=("$2"); shift 2 ;;
        --) shift; break ;;
        *) break ;;
        esac
    done
    if ! command -v expect >/dev/null 2>&1; then
        CODE=skip OUT="" ERR=""
        printf '\n[skip] %s$ gage %s (no expect)\n' "$who" "$*" >>"$QA_S/log/transcript.txt"
        return 0
    fi
    _run "$who" "" "QA_DIALOGUE=$dlg" "TERM=xterm" "${extra[@]+"${extra[@]}"}" -- expect -f "$QA_DIR/tty.exp" "$GAGE" "$@"
    OUT="$(tr -d '\r' <<<"$OUT")"
    printf '%s\t%s\t%s\n' "$QA_SCENARIO" "$(_command_name "$@")" "$CODE" >>"$QA_COVERAGE"
}

# dialogue <regex> <reply> [<regex> <reply>]...: a gtty dialogue.
dialogue() {
    while (($# >= 2)); do
        printf '%s\t%s\n' "$1" "$2"
        shift 2
    done
}

# PROMPT_PW: the regex every unlock prompt matches.
PROMPT_PW='Enter passphrase[^:]*: '

# as <persona> [-i STDIN] [-e K=V]... [--] <cmd...>: any other program
# (git, ls, expect) on that persona's machine. Sets OUT, ERR and CODE.
as() {
    local who="$1" input="" extra=()
    shift
    while (($#)); do
        case "$1" in
        -i) input="$2"; shift 2 ;;
        -e) extra+=("$2"); shift 2 ;;
        --) shift; break ;;
        *) break ;;
        esac
    done
    _run "$who" "$input" "${extra[@]+"${extra[@]}"}" -- "$@"
}

_run() {
    local who="$1" input="$2" env=() e
    shift 2
    while [[ "$1" != "--" ]]; do env+=("$1"); shift; done
    shift
    local base=()
    while IFS= read -r e; do base+=("$e"); done < <(_persona_env "$who")
    QA_N=$((QA_N + 1))
    local o="$QA_S/out/$QA_N.out" r="$QA_S/out/$QA_N.err"
    (cd "$(persona_home "$who")/home" &&
        printf '%s' "$input" | env -i "${base[@]}" "${env[@]+"${env[@]}"}" "$@" >"$o" 2>"$r")
    CODE=$?
    OUT="$(cat "$o")"
    ERR="$(cat "$r")"
    {
        local shown=("$@")
        [[ "${shown[0]}" == "$GAGE" ]] && shown[0]=gage
        printf '\n[%d] %s$ %s\n' "$QA_N" "$who" "$(_redact "${shown[*]}")"
        ((${#env[@]})) && printf '    env: %s\n' "$(_redact "${env[*]}")"
        [[ -n "$input" ]] && printf '    stdin: %q\n' "$(_redact "$input")"
        printf '    exit: %s\n' "$CODE"
        [[ -n "$OUT" ]] && printf '%s\n' "$(_redact "$OUT")" | sed 's/^/    out| /'
        [[ -n "$ERR" ]] && printf '%s\n' "$(_redact "$ERR")" | sed 's/^/    err| /'
    } >>"$QA_S/log/transcript.txt"
    return 0
}

# _record <ok:0|1> <assertion> [detail]
_record() {
    local status=PASS
    if [[ "$CODE" == skip ]]; then
        status=SKIP
    elif (($1)); then
        status=FAIL
        QA_FAILS=$((QA_FAILS + 1))
        printf '  FAIL [%s] %s\n' "$QA_STEP" "$2" >&2
        [[ -n "${3:-}" ]] && printf '%s\n' "$3" | sed 's/^/       /' >&2
    elif [[ "$status" == PASS ]]; then
        QA_PASSES=$((QA_PASSES + 1))
    fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$QA_SCENARIO" "$QA_STEP" "$2" "$status" "[$QA_N]" >>"$QA_RESULTS"
    printf '    %s: %s\n' "$status" "$2" >>"$QA_S/log/transcript.txt"
}

_last() {
    printf 'exit %s\nstdout: %s\nstderr: %s' "$CODE" "$(_redact "$OUT" | head -c 600)" "$(_redact "$ERR" | head -c 600)"
}

# expect_exit <code>: the last command exited <code>.
expect_exit() {
    [[ "$CODE" == "$1" ]]
    _record $? "exit $1 (got $CODE)" "$(_last)"
}

# expect_out / expect_err <ERE>: stdout / stderr matches.
expect_out() {
    grep -Eq -- "$1" <<<"$OUT"
    _record $? "stdout ~ /$1/" "$(_last)"
}
expect_err() {
    grep -Eq -- "$1" <<<"$ERR"
    _record $? "stderr ~ /$1/" "$(_last)"
}

# expect_no_out / expect_no_err <ERE>: stdout / stderr does not match.
expect_no_out() {
    ! grep -Eq -- "$1" <<<"$OUT"
    _record $? "stdout !~ /$1/" "$(_last)"
}
expect_no_err() {
    ! grep -Eq -- "$1" <<<"$ERR"
    _record $? "stderr !~ /$1/" "$(_last)"
}

# expect_true <description> <cmd...>: an arbitrary check, for files and
# repositories rather than gage's output.
expect_true() {
    local d="$1"
    shift
    "$@" >/dev/null 2>&1
    _record $? "$d"
}

# bare_remote <name>: an empty bare repo on branch main, echoing its path.
bare_remote() {
    local r="$QA_S/remotes/$1.git"
    git init -q --bare -b main "$r"
    printf '%s' "$r"
}

# vault_dir <persona> <vault>: where gage put that persona's clone.
vault_dir() { printf '%s/data/gage/vaults/%s' "$(persona_home "$1")" "$2"; }

# entry_id <title>: the id of the entry titled <title> in the last `ls`.
entry_id() {
    awk -v t="$1" '$0 ~ t { for (i = 1; i <= NF; i++) if ($i ~ /^[0-9a-f]{8}(-|$)/) { print $i; exit } }' <<<"$OUT"
}

# new_vault <persona> <vault> <remote-or-""> [extra init args...]: init a
# vault as that persona, with its fixed passphrase and no recovery key
# unless the extra args ask for one. Asserts it worked.
new_vault() {
    local who="$1" v="$2" remote="$3" args=()
    shift 3
    [[ -n "$remote" ]] && args+=(--remote "$remote")
    local rk=(--no-recovery-key)
    local a
    for a in "$@"; do [[ "$a" == --recovery-key-out ]] && rk=(); done
    g "$who" -i "$(lines "$(pw "$who")" "$(pw "$who")")" -- \
        init "$v" --device "$who-laptop" "${args[@]+"${args[@]}"}" "${rk[@]+"${rk[@]}"}" "$@"
    expect_exit 0
}

# put <persona> <title> <value> [-u vault]: insert an entry, asserting it.
put() {
    local who="$1" t="$2" v="$3"
    shift 3
    g "$who" -i "$(pw "$who")"$'\n'"$v" -- insert "$t" --value-stdin "$@"
    expect_exit 0
}

# unlock <persona> [more lines...]: stdin that answers one unlock prompt,
# then whatever else the command will ask.
unlock() {
    local who="$1"
    shift
    lines "$(pw "$who")" "$@"
}

# finish: summarise and exit non-zero if anything failed.
finish() {
    printf '   %d passed, %d failed  (transcript: %s)\n' "$QA_PASSES" "$QA_FAILS" "$QA_S/log/transcript.txt"
    ((QA_FAILS == 0))
}
