#!/usr/bin/env bash
# Build Rust core for all Android ABIs and copy .so into jniLibs
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$REPO_ROOT/rust-core"
JNILIBS_DIR="$REPO_ROOT/expo-module/android/src/main/jniLibs"

# ── NDK setup ──────────────────────────────────────────────────────────────────
NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK:-}}"
if [[ -z "$NDK" ]]; then
  # Common SDK locations
  for candidate in \
      "$HOME/Android/Sdk/ndk/$(ls "$HOME/Android/Sdk/ndk" 2>/dev/null | sort -V | tail -1)" \
      "$HOME/Library/Android/sdk/ndk/$(ls "$HOME/Library/Android/sdk/ndk" 2>/dev/null | sort -V | tail -1)"; do
    [[ -d "$candidate" ]] && NDK="$candidate" && break
  done
fi
if [[ -z "$NDK" || ! -d "$NDK" ]]; then
  echo "ERROR: Android NDK not found. Set ANDROID_NDK_HOME." >&2
  exit 1
fi
echo "NDK: $NDK"

TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
if [[ ! -d "$TOOLCHAIN" ]]; then
  TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin"
fi
if [[ ! -d "$TOOLCHAIN" ]]; then
  echo "ERROR: NDK LLVM toolchain not found at $TOOLCHAIN" >&2
  exit 1
fi
echo "Toolchain: $TOOLCHAIN"
export PATH="$TOOLCHAIN:$PATH"

# ── Tell cc-rs exactly which compiler to use for each target ──────────────────
# cc-rs ignores .cargo/config.toml `linker` — needs CC_<target> env vars.
# NDK uses armv7a prefix for arm32, NOT the Rust target triple prefix.
AR="$TOOLCHAIN/llvm-ar"
export CC_aarch64_linux_android="$TOOLCHAIN/aarch64-linux-android24-clang"
export AR_aarch64_linux_android="$AR"
export CC_armv7_linux_androideabi="$TOOLCHAIN/armv7a-linux-androideabi24-clang"
export AR_armv7_linux_androideabi="$AR"
export CC_x86_64_linux_android="$TOOLCHAIN/x86_64-linux-android24-clang"
export AR_x86_64_linux_android="$AR"
export CC_i686_linux_android="$TOOLCHAIN/i686-linux-android24-clang"
export AR_i686_linux_android="$AR"

# ── Targets: (rust-target, ABI-folder) ────────────────────────────────────────
declare -A TARGETS=(
  [aarch64-linux-android]="arm64-v8a"
  [armv7-linux-androideabi]="armeabi-v7a"
  [x86_64-linux-android]="x86_64"
)

cd "$RUST_DIR"

for TARGET in "${!TARGETS[@]}"; do
  ABI="${TARGETS[$TARGET]}"
  echo ""
  echo "▶ Building $TARGET → $ABI"
  cargo build --release --target "$TARGET"

  DEST="$JNILIBS_DIR/$ABI"
  mkdir -p "$DEST"
  cp "target/$TARGET/release/liblxmf_rn.so" "$DEST/liblxmf_rn.so"
  echo "  ✓ Copied → $DEST/liblxmf_rn.so"
done

echo ""
echo "✅ Android build complete."
