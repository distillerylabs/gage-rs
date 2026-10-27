#!/bin/sh
# Proves the library-purity lint (crates/gage/clippy.toml, plus the denies
# in crates/gage/src/lib.rs) still fires. crates/gage must never print, touch
# stdin/stdout/stderr, exit the process, or spawn a subprocess; a lint that
# quietly stopped firing (a renamed path, a dropped deny) would let that
# regress silently forever, so this plants each forbidden item in a scratch copy of the
# workspace and requires clippy to reject it. It also requires that a test
# can opt out locally, and that the untouched crate passes.
#
# Runs in CI's lint job on every platform: `make lint-selftest`.

set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# A native cargo needs a native path; Git Bash's /tmp/... is not one.
if command -v cygpath >/dev/null 2>&1; then
	tmp="$(cygpath -m "$tmp")"
fi

cp "$root/Cargo.toml" "$root/Cargo.lock" "$root/rust-toolchain.toml" "$tmp/"
cp -R "$root/crates" "$tmp/crates"
rm -rf "$tmp/crates/gage-cli/target" "$tmp/crates/gage/target"
lib="$tmp/crates/gage/src/lib.rs"
cp "$lib" "$tmp/lib.rs.orig"

# One scratch target dir for every probe: dependencies build once, and each
# probe only recompiles the one crate it edited.
CARGO_TARGET_DIR="$tmp/target"
export CARGO_TARGET_DIR

failures=0

# probe <name> <expect: pass|fail> <source appended to lib.rs>
probe() {
	name="$1"
	expect="$2"
	source="$3"
	cp "$tmp/lib.rs.orig" "$lib"
	printf '\n%s\n' "$source" >>"$lib"
	if out="$(cd "$tmp" && cargo clippy -p gage --all-targets --locked -- -D warnings 2>&1)"; then
		result=pass
	else
		result=fail
	fi
	# A failure only counts if clippy rejected it as disallowed, not because
	# the probe itself didn't compile.
	if [ "$result" = fail ] && [ "$expect" = fail ] && ! printf '%s' "$out" | grep -q 'disallowed'; then
		echo "FAIL  $name: clippy failed, but not with a disallowed-item error:"
		printf '%s\n' "$out"
		failures=$((failures + 1))
	elif [ "$result" != "$expect" ]; then
		echo "FAIL  $name: expected clippy to $expect, but it did $result"
		printf '%s\n' "$out"
		failures=$((failures + 1))
	else
		echo "ok    $name"
	fi
}

probe "the untouched crate passes" pass ""
probe "print!" fail 'pub fn probe() { print!("x"); }'
probe "println!" fail 'pub fn probe() { println!("x"); }'
probe "eprint!" fail 'pub fn probe() { eprint!("x"); }'
probe "eprintln!" fail 'pub fn probe() { eprintln!("x"); }'
probe "dbg!" fail 'pub fn probe() { let _ = dbg!(1); }'
probe "std::io::stdin" fail 'pub fn probe() { let _ = std::io::stdin(); }'
probe "std::io::stdout" fail 'pub fn probe() { let _ = std::io::stdout(); }'
probe "std::io::stderr" fail 'pub fn probe() { let _ = std::io::stderr(); }'
probe "std::process::exit" fail 'pub fn probe() { std::process::exit(1); }'
probe "std::process::Command::new" fail 'pub fn probe() { let _ = std::process::Command::new("git"); }'
probe "Command::new through a use" fail 'use std::process::Command as Probe;
pub fn probe() { let _ = Probe::new("git"); }'
probe "a test may opt out locally" pass '#[cfg(test)]
mod probe_tests {
    #[test]
    #[allow(clippy::disallowed_macros)]
    fn prints() {
        println!("x");
    }
}'

if [ "$failures" -ne 0 ]; then
	echo "lint-selftest: $failures probe(s) failed" >&2
	exit 1
fi
echo "lint-selftest: all probes behaved"
