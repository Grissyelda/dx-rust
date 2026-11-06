#!/bin/bash
# Installs what build_linux.sh needs on Ubuntu / Debian (WSL or a Linux PC).
#   setup_linux.sh root   the system packages (run as root)
#   setup_linux.sh user   Rust, zig and cargo-zigbuild for the user who builds
set -euo pipefail
case "${1:-}" in
  root)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y build-essential pkg-config curl xz-utils python3-pip \
      libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev libx11-dev libxi-dev libxcursor-dev \
      libxrandr-dev libgtk-3-dev
    ;;
  user)
    if ! command -v cargo >/dev/null && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    fi
    . "$HOME/.cargo/env"
    rustup update stable
    rustup default stable
    # zig (cargo-zigbuild runs `python3 -m ziglang` when there's no zig on the PATH).
    python3 -m pip install --user --upgrade --break-system-packages ziglang 2>/dev/null \
      || python3 -m pip install --user --upgrade ziglang
    cargo install --locked cargo-zigbuild
    echo "Linux build tools ready: $(cargo --version), $(cargo zigbuild --version 2>/dev/null | head -1)"
    ;;
  *) echo "usage: setup_linux.sh root|user"; exit 2 ;;
esac
