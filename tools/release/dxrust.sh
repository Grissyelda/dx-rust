#!/bin/sh
# DX RUST for Steam Deck / Linux: starts the game straight away (good for Game Mode).
# Run ./dxlauncher instead to choose the game folder, display mode or Lay D Denton.
cd "$(dirname "$0")" || exit 1
exec ./dxview "$@"
