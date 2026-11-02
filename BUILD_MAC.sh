#!/bin/bash
# Builds a shareable copy for Apple Silicon: dist/DX RUST.app (the launcher and the game) and dist/dxrust-macos.zip.
# No game files are copied: the player picks their own Deus Ex folder in the launcher.
# RUST_TOOLCHAIN=<name> selects a rustup toolchain (Bevy needs Rust 1.95 or later).
set -e
cd "$(dirname "$0")"
VERSION="${1:-$(git describe --tags --always 2>/dev/null || echo dev)}"
VERSION="${VERSION#v}"
TOOLCHAIN=()
[ -n "$RUST_TOOLCHAIN" ] && TOOLCHAIN=("+$RUST_TOOLCHAIN")

# Own build folders, as in BUILD_RELEASE.bat: what is copied below always comes from these two builds, whatever
# CARGO_TARGET_DIR is set to.
ROOT="$PWD"
CARGO_TARGET_DIR="$ROOT/target/dist-build" cargo "${TOOLCHAIN[@]}" build --release -p dxview
(cd crates/dxlauncher && CARGO_TARGET_DIR="$ROOT/target/dist-launcher" cargo "${TOOLCHAIN[@]}" build --release)

APP="dist/DX RUST.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/dist-build/release/dxview target/dist-launcher/release/dxlauncher "$APP/Contents/MacOS/"
sed "s/VERSION/$VERSION/g" tools/macos/Info.plist > "$APP/Contents/Info.plist"
cp LICENSE README.md THIRD_PARTY.md "$APP/Contents/Resources/"
# Relief maps for Remastered Lighting: see BUILD_RELEASE.bat. The game looks for them beside its executable.
[ -d relief ] && cp -R relief "$APP/Contents/MacOS/relief"

ICONSET="$(mktemp -d "${TMPDIR:-/tmp}/dxrust-icon.XXXXXX")/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 64 128 256; do
  sips -z "$size" "$size" assets/dxrust_256.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
done
sips -z 64 64 assets/dxrust_256.png --out "$ICONSET/icon_32x32@2x.png" >/dev/null
sips -z 256 256 assets/dxrust_256.png --out "$ICONSET/icon_128x128@2x.png" >/dev/null
rm "$ICONSET/icon_64x64.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$ICONSET")"

# An ad-hoc signature: an Apple Silicon Mac runs nothing unsigned, and this one needs no developer account.
codesign --force --deep --sign - "$APP"
rm -f dist/dxrust-macos.zip
ditto -c -k --keepParent "$APP" dist/dxrust-macos.zip
echo "Built $APP and dist/dxrust-macos.zip"
