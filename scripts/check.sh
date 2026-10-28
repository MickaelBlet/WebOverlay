#!/usr/bin/env bash
# Pre-commit checks in Docker: cargo fmt --check, clippy (Linux + Windows/cargo-xwin), tests.
# Usage: scripts/check.sh
set -euo pipefail
cd "$(dirname "$0")/.."

docker buildx bake --progress=plain check
