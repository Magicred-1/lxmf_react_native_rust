#!/usr/bin/env bash
# Build Rust core for iOS and create an XCFramework static lib
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
  echo "ERROR: iOS build requires macOS." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$REPO_ROOT/rust-core"
IOS_OUT="$REPO_ROOT/expo-module/ios/RustCore"

mkdir -p "$IOS_OUT"

cd "$RUST_DIR"

# ── Targets ───────────────────────────────────────────────────────────────────
DEVICE_TARGET="aarch64-apple-ios"
SIM_ARM_TARGET="aarch64-apple-ios-sim"
SIM_X86_TARGET="x86_64-apple-ios"

echo "▶ Building device ($DEVICE_TARGET)"
cargo build --release --target "$DEVICE_TARGET"

echo "▶ Building sim arm64 ($SIM_ARM_TARGET)"
cargo build --release --target "$SIM_ARM_TARGET"

echo "▶ Building sim x86_64 ($SIM_X86_TARGET)"
cargo build --release --target "$SIM_X86_TARGET"

# ── Fat lib for simulator (arm64 + x86_64) ────────────────────────────────────
SIM_FAT="$RUST_DIR/target/sim-fat/liblxmf_rn.a"
mkdir -p "$(dirname "$SIM_FAT")"
echo "▶ Lipo sim fat lib"
lipo -create \
  "target/$SIM_ARM_TARGET/release/liblxmf_rn.a" \
  "target/$SIM_X86_TARGET/release/liblxmf_rn.a" \
  -output "$SIM_FAT"

# ── XCFramework ───────────────────────────────────────────────────────────────
XCFW="$IOS_OUT/liblxmf_rn.xcframework"
rm -rf "$XCFW"
echo "▶ Creating XCFramework → $XCFW"
xcodebuild -create-xcframework \
  -library "target/$DEVICE_TARGET/release/liblxmf_rn.a" \
  -library "$SIM_FAT" \
  -output "$XCFW"

echo ""
echo "✅ iOS build complete: $XCFW"
