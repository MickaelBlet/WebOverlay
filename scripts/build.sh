#!/usr/bin/env bash
# Build the desktop app for Linux amd64 and/or Windows amd64 in Docker.
# Usage: scripts/build.sh [--fast] [linux|windows]...   (default: both)
#   --fast: thin LTO + 16 codegen units, faster rebuilds, slightly bigger binaries.
# Output: dist-tauri/linux/{*.deb,*.rpm,*.AppImage}, dist-tauri/windows/{*-setup.exe,*-portable.exe}
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  echo "usage: $0 [--fast] [linux|windows]..." >&2
  exit 2
}

fast=
targets=()
for arg in "$@"; do
  case "$arg" in
    --fast) fast=1 ;;
    linux | windows) targets+=("$arg") ;;
    -h | --help) usage ;;
    *) echo "unknown argument: $arg" >&2; usage ;;
  esac
done
[ ${#targets[@]} -gt 0 ] || targets=(linux windows)

for t in "${targets[@]}"; do rm -rf "dist-tauri/$t"; done
FAST=$fast docker buildx bake "${targets[@]}"
for t in "${targets[@]}"; do ls -lh "dist-tauri/$t"; done
