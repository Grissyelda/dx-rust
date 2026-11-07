#!/usr/bin/env bash
# DX RUST for Android (Classic): dist/dxrust-android.apk, arm64, Android 10 or newer.
#
# Needs the Android SDK (a platform and build-tools) and NDK, cargo-apk and Rust's aarch64-linux-android target.
# Paths: ANDROID_HOME (or ANDROID_SDK_ROOT) and ANDROID_NDK_ROOT (or ANDROID_NDK_HOME / ANDROID_NDK_LATEST_HOME, else
# the newest NDK under $ANDROID_HOME/ndk). DXR_ROOT = the source folder (default: two folders up from this script),
# DXR_TARGET = cargo's target folder.
#
# Signing: with DXR_KEYSTORE (+ DXR_KEYSTORE_PASSWORD) the APK is signed with that key; without it, with this
# machine's own key (made on first use, below). A newer build installs over an older one only with the same key.
set -euo pipefail
ROOT=${DXR_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}
export ANDROID_HOME=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}
export ANDROID_SDK_ROOT=$ANDROID_HOME
if [ -z "${ANDROID_NDK_ROOT:-}" ]; then
    ANDROID_NDK_ROOT=${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)}}
fi
export ANDROID_NDK_ROOT ANDROID_NDK_HOME=$ANDROID_NDK_ROOT
echo "SDK: $ANDROID_HOME"
echo "NDK: $ANDROID_NDK_ROOT"
[ -d "$ANDROID_NDK_ROOT" ] || { echo "no Android NDK found (set ANDROID_NDK_ROOT)"; exit 1; }
rustup target add aarch64-linux-android >/dev/null
command -v cargo-apk >/dev/null || cargo install cargo-apk --locked
# Without a key given, this machine's own one (made once in ~/.android/dxrust-release.keystore): builds from the
# same machine install over each other. Keep that file - an app signed with another key can't update it.
if [ -z "${DXR_KEYSTORE:-}" ]; then
    DXR_KEYSTORE=$HOME/.android/dxrust-release.keystore
    DXR_KEYSTORE_PASSWORD=${DXR_KEYSTORE_PASSWORD:-dxrust}
    if [ ! -f "$DXR_KEYSTORE" ]; then
        mkdir -p "$(dirname "$DXR_KEYSTORE")"
        keytool -genkeypair -keystore "$DXR_KEYSTORE" -storepass "$DXR_KEYSTORE_PASSWORD" -keypass "$DXR_KEYSTORE_PASSWORD" \
            -alias dxrust -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=DX RUST" >/dev/null
        echo "made a signing key: $DXR_KEYSTORE"
    fi
fi
export CARGO_APK_RELEASE_KEYSTORE=$DXR_KEYSTORE
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=${DXR_KEYSTORE_PASSWORD:-android}
cd "$ROOT"
TARGET=${DXR_TARGET:-$ROOT/target}
if [ "${DXR_FAST:-0}" = "1" ]; then
    export CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
fi
cargo apk build --release -p dxandroid --target-dir "$TARGET"
mkdir -p dist
cp "$TARGET/release/apk/dxrust.apk" dist/dxrust-android.apk
ls -l dist/dxrust-android.apk
