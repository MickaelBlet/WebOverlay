# Building

## Prerequisites

Release builds and checks: Docker with buildx only (any Linux).

Dev run (`cargo run`) on Linux:

```sh
sudo apt install pkg-config build-essential libssl-dev libwebkit2gtk-4.1-dev \
  libsoup-3.0-dev libjavascriptcoregtk-4.1-dev librsvg2-dev libayatana-appindicator3-dev \
  libdbus-1-dev libxdo-dev
```

Windows: Rust (MSVC toolchain) and WebView2 (preinstalled on Windows 10/11).

## Development

```sh
cargo run -- --config examples/config.toml        # dev run
cargo test                                        # unit tests
scripts/check.sh                                  # fmt, clippy (Linux + Windows), tests in Docker
```

## Release builds

`scripts/build.sh` (`docker buildx bake`) builds both platforms in parallel from any Linux, only
Docker (with buildx) required. `scripts/build.sh [--fast] [linux|windows]...` builds only the given
platforms; `--fast` trades a slightly bigger binary for a much quicker rebuild (thin LTO):

- `dist-tauri/linux/`: `.deb`, `.rpm` (glibc 2.35+ with `webkit2gtk-4.1`, e.g. Ubuntu 22.04,
  Debian 12), `.AppImage` (self-contained: bundles WebKitGTK, GTK and glibc, runs on older distros
  such as RHEL/Rocky 8)
- `dist-tauri/windows/`: NSIS `*-setup.exe` and `WebOverlay-portable.exe` (cargo-xwin, MSVC target)

Release Windows builds have no console: `--list`, `--help` and errors print nothing there.

## Documentation screenshots

`scripts/screenshots.sh` regenerates `docs/images/*.svg` (light, and `*-dark.svg`): the real
`dist/` pages are rendered in headless Chromium with a stub of the app IPC and fixture data
(`docs/screenshots/`: fake site, icons, `generate.mjs` scenes), printed to PDF, then converted to
SVG with text as paths (no font needed to view them). Rerun it after a UI change.

Needs Node 22+, Chromium or `chrome-headless-shell` (`CHROME=<path>`, else found on `PATH` or in the
Playwright cache), Python 3 with PyMuPDF (`pip install pymupdf`, or `PYTHON=<python>`) and cargo
(the editor completion docs come from `src/schema.rs`). `SCENES=editor,vault` renders only those.
