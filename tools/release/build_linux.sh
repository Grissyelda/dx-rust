#!/bin/bash
# Builds the Linux / Steam Deck release: <out>/dxrust-steamdeck.tar.xz (dxview, dxlauncher, dxrust.sh and the
# notices; nothing of the game's). Runs on any x86_64 Linux with the tools from setup_linux.sh, including WSL on a
# Windows PC (the DX RUST Release Builder runs it there) and GitHub's Ubuntu runners.
# The binaries are linked against glibc 2.35 (cargo-zigbuild), so they start on SteamOS and on older distros too.
#   build_linux.sh [out folder]        default: dist/ in the project
#   DXR_TARGET=<dir>                   build folder (default ~/.dxrust-build: keep it off /mnt/c, it's much faster)
#   DXR_ROOT=<dir>                     the project folder (default: two folders up from the script)
#   DXR_FAST=1                         thin LTO (a quicker, slightly slower-running build)
set -euo pipefail
# DXR_ROOT = the project folder (needed when the script comes in on stdin, as the builder runs it).
cd "${DXR_ROOT:-$(dirname "$0")/../..}"
ROOT="$PWD"
OUT="${1:-$ROOT/dist}"
TD="${DXR_TARGET:-$HOME/.dxrust-build}"
TRIPLE=x86_64-unknown-linux-gnu
GLIBC=2.35
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
command -v cargo-zigbuild >/dev/null || { echo "cargo-zigbuild is missing: run setup_linux.sh first"; exit 1; }
mkdir -p "$OUT" "$TD"

# zig's C compiler uses its own glibc headers and not /usr/include, so hidapi's one system header goes in a folder
# of its own (adding all of /usr/include would clash with zig's libc).
INC="$TD/include"
mkdir -p "$INC"
cp /usr/include/libudev.h "$INC/"
export CFLAGS_x86_64_unknown_linux_gnu="-I$INC"
# The system's own .so files (ALSA, udev, GTK) may name newer glibc versions than 2.35; the Deck brings its own.
export RUSTFLAGS="-C link-arg=-Wl,--allow-shlib-undefined"
if [ "${DXR_FAST:-0}" = 1 ]; then
  export CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
fi

echo "== game (dxview) =="
CARGO_TARGET_DIR="$TD/game" cargo zigbuild --release -p dxview --target "$TRIPLE.$GLIBC"
echo "== launcher (dxlauncher) =="
(cd crates/dxlauncher && CARGO_TARGET_DIR="$TD/launcher" cargo zigbuild --release --target "$TRIPLE.$GLIBC")

echo "== package =="
STAGE="$TD/stage"
rm -rf "$STAGE"
mkdir -p "$STAGE/dxrust"
cp "$TD/game/$TRIPLE/release/dxview" "$TD/launcher/$TRIPLE/release/dxlauncher" "$STAGE/dxrust/"
cp tools/release/dxrust.sh "$STAGE/dxrust/"
cp LICENSE "$STAGE/dxrust/LICENSE.txt"
cp README.md THIRD_PARTY.md "$STAGE/dxrust/"
sed -i 's/\r$//' "$STAGE/dxrust/dxrust.sh"
chmod +x "$STAGE/dxrust/dxview" "$STAGE/dxrust/dxlauncher" "$STAGE/dxrust/dxrust.sh"
rm -f "$OUT/dxrust-steamdeck.tar.xz"
tar -C "$STAGE" -cJf "$OUT/dxrust-steamdeck.tar.xz" dxrust
echo "Built $OUT/dxrust-steamdeck.tar.xz ($(du -h "$OUT/dxrust-steamdeck.tar.xz" | cut -f1))"
