#!/bin/sh
# Runs the Swift smoke test (crates/gage-ffi/tests/swift/main.swift) in an
# iOS simulator, against the XCFramework `make xcframework` built. `make
# ios-smoke` runs it.
#
# It makes a throwaway simulator and deletes it on the way out, so it
# neither depends on nor disturbs the simulators already on the machine.

set -eu

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

MIN_OS="${IPHONEOS_DEPLOYMENT_TARGET:-18.0}"
fw="$root/target/ios/GageFFI.xcframework"
swift_out="$root/target/ios/swift"

die() {
	echo "ios-smoke: $*" >&2
	exit 1
}

[ -d "$fw" ] || die "$fw is missing: run 'make xcframework' first"
xcrun --find simctl >/dev/null 2>&1 || die "Xcode's simctl is not available (set DEVELOPER_DIR?)"

work="$(mktemp -d)"
udid=""
cleanup() {
	if [ -n "$udid" ]; then
		xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
		xcrun simctl delete "$udid" >/dev/null 2>&1 || true
	fi
	rm -rf "$work"
}
trap cleanup EXIT

# The vault the smoke reads, written on the host through the library.
cargo run --locked -q -p gage-ffi --example smoke_fixture -- "$work/root" >/dev/null

slice="$(find "$fw" -type d -name '*simulator*' -maxdepth 1 | head -n 1)"
[ -n "$slice" ] || die "no simulator slice in $fw"
xcrun --sdk iphonesimulator swiftc -O -target "arm64-apple-ios$MIN_OS-simulator" \
	-o "$work/smoke" \
	"$root/crates/gage-ffi/tests/swift/main.swift" "$swift_out/gage_ffi.swift" \
	-I "$slice/Headers" -Xcc "-fmodule-map-file=$slice/Headers/module.modulemap" \
	-L "$slice" -lgage_ffi -liconv -framework Security -framework CoreFoundation

# The first iPhone the newest installed iOS runtime supports: its section
# header gives the runtime ("-- iOS 27.0 --"), its name the device type.
pick="$(xcrun simctl list devices available | awk '
	/^-- iOS [0-9.]+ --$/ { ver = $3; gsub(/\./, "-", ver); next }
	/^-- / { ver = ""; next }
	ver != "" && /^ +iPhone / {
		name = $0; sub(/^ +/, "", name); sub(/ \([0-9A-F-]{36}\).*$/, "", name)
		print "com.apple.CoreSimulator.SimRuntime.iOS-" ver "|" name; exit
	}')"
[ -n "$pick" ] || die "no iPhone simulator on an installed iOS runtime"
runtime="${pick%%|*}"
devtype="${pick#*|}"

udid="$(xcrun simctl create gage-smoke "$devtype" "$runtime")"
xcrun simctl boot "$udid"
xcrun simctl bootstatus "$udid" -b >/dev/null

SIMCTL_CHILD_ROOT="$work/root" xcrun simctl spawn "$udid" "$work/smoke"
