#!/usr/bin/env bash
# Build the desktop app for Linux amd64 and Windows amd64 in Docker.
# Output: dist-tauri/linux/{*.deb,*.rpm,*.AppImage}, dist-tauri/windows/{*-setup.exe,*-portable.exe}
set -euo pipefail
cd "$(dirname "$0")/.."

rm -rf dist-tauri
docker buildx bake
ls -lh dist-tauri/*
