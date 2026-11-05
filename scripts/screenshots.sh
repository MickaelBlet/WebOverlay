#!/bin/sh
# Regenerates the documentation screenshots (docs/images/*.svg, light + *-dark.svg) from the
# real dist/ pages, rendered in headless Chromium with fixture data (docs/screenshots/).
#
# Needs: Node 22+, Chromium or chrome-headless-shell (CHROME=<path>, else found on PATH or in
# the Playwright cache), Python 3 with PyMuPDF (PYTHON=<python with pymupdf>), cargo.
# usage: scripts/screenshots.sh            (SCENES=editor,vault to render only some)
set -eu
cd "$(dirname "$0")/.."

if [ -z "${CHROME:-}" ]; then
  for c in chrome-headless-shell chromium chromium-browser google-chrome; do
    CHROME=$(command -v "$c" 2>/dev/null) && break
  done
  [ -n "${CHROME:-}" ] || CHROME=$(ls -d "$HOME"/.cache/ms-playwright/chromium_headless_shell-*/chrome-headless-shell-linux64/chrome-headless-shell 2>/dev/null | tail -1)
fi
[ -x "${CHROME:-}" ] || { echo "Chromium not found: set CHROME" >&2; exit 1; }
PYTHON=${PYTHON:-python3}
"$PYTHON" -c "import pymupdf" 2>/dev/null || { echo "PyMuPDF missing: pip install pymupdf (or set PYTHON)" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'kill "$server" 2>/dev/null; rm -rf "$tmp"' EXIT

# Config key docs of the editor completion, as served by `editor_schema`.
WEBOVERLAY_SCHEMA_OUT="$tmp/schema.json" cargo test --quiet schema_serializes >/dev/null

port=${PORT:-8731}
python3 -m http.server "$port" --bind 127.0.0.1 >/dev/null 2>&1 &
server=$!
sleep 1

CHROME="$CHROME" node docs/screenshots/generate.mjs "http://127.0.0.1:$port" "$tmp" "$tmp/schema.json"
"$PYTHON" docs/screenshots/to-svg.py "$tmp"/*.pdf >/dev/null
cp "$tmp"/*.svg docs/images/
ls -1 "$tmp"/*.svg | sed 's|.*/|docs/images/|'
