#!/bin/sh
# Builds target/ios/GageFFI.xcframework (device and simulator slices) and
# the generated Swift in target/ios/swift/. `make xcframework` runs it.
#
# The deployment target is set for every slice, C objects included: without
# it the vendored C libraries are built for the SDK version and an app
# linking them claims that as its minimum (spike findings, item 3). The
# script then reads the minimum OS back out of every object and fails if
# any is newer. Older is fine (compiler_builtins' atomics ship at 14.0 on
# the simulator and cannot raise an app's minimum), but at least one object
# must carry exactly the target, so a build that ignored it can't pass.

set -eu

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

MIN_OS="${IPHONEOS_DEPLOYMENT_TARGET:-18.0}"
export IPHONEOS_DEPLOYMENT_TARGET="$MIN_OS"
out="$root/target/ios"
device=aarch64-apple-ios
sim=aarch64-apple-ios-sim

die() {
	echo "build-xcframework: $*" >&2
	exit 1
}

xcrun --find xcodebuild >/dev/null 2>&1 ||
	die "Xcode is not available: install Xcode and run 'sudo xcode-select -s /Applications/Xcode.app' (or set DEVELOPER_DIR)"
for t in $device $sim; do
	rustup target list --installed | grep -qx "$t" ||
		die "missing Rust target $t: run 'rustup target add $t'"
done

for t in $device $sim; do
	cargo build --locked --release --target "$t" -p gage-ffi
done

rm -rf "$out"
mkdir -p "$out/headers" "$out/swift"

# Bindings come from the library itself, so they can't drift from it.
cargo run --locked -q -p gage-ffi --features bindgen --bin uniffi-bindgen -- \
	generate --library "target/$device/release/libgage_ffi.a" \
	--language swift --out-dir "$out/swift"

# The C header and module map go in the framework; the Swift stays outside.
cp "$out/swift/gage_ffiFFI.h" "$out/headers/"
cp "$out/swift/gage_ffiFFI.modulemap" "$out/headers/module.modulemap"

xcodebuild -create-xcframework \
	-library "target/$device/release/libgage_ffi.a" -headers "$out/headers" \
	-library "target/$sim/release/libgage_ffi.a" -headers "$out/headers" \
	-output "$out/GageFFI.xcframework"

# No object in either archive may claim a newer minimum than the target.
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
bad=0
for t in $device $sim; do
	mkdir "$scratch/$t"
	(cd "$scratch/$t" && ar x "$root/target/$t/release/libgage_ffi.a")
	found="$(find "$scratch/$t" -name '*.o' -exec xcrun vtool -show-build {} \; 2>/dev/null |
		awk '$1 == "minos" { print $2 }' | sort -u)"
	[ -n "$found" ] || die "$t: no object reports a minimum OS"
	newest="$(printf '%s\n' "$found" | sort -V | tail -n 1)"
	if [ "$newest" != "$MIN_OS" ]; then
		echo "build-xcframework: $t has objects with minimum OS $newest, newest wanted $MIN_OS (found: $(echo $found))" >&2
		bad=1
	fi
done
[ "$bad" = 0 ] || exit 1

echo "built $out/GageFFI.xcframework (iOS $MIN_OS) and $out/swift/"
