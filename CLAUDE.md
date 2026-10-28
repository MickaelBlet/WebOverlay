# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

WebOverlay: Tauri 2 desktop app (pure Rust, no npm, no UI) that opens websites listed in a TOML config as standalone "webapp" windows and injects user JS/CSS into every page. Targets Linux and Windows.

## Commands

```sh
cargo run -- --config examples/config.toml demo   # dev run (WSLg works)
cargo run -- --config examples/config.toml --list
cargo test                                        # unit tests (config parsing, script generation)
cargo test wildcards                              # one test by name filter
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo build --release --target x86_64-pc-windows-gnu   # quick Windows exe from WSL (needs WebView2Loader.dll beside it)
cargo clippy --target x86_64-pc-windows-gnu -- -D warnings   # type-check the cfg(windows) code locally
scripts/build.sh                                  # release bundles in Docker -> dist-tauri/{linux,windows}/
```

- Linux builds need the WebKitGTK dev packages listed in README.md.
- `scripts/build.sh` = `docker buildx bake` (Dockerfile modeled on ../ProjectScaffold-Viewer): Linux deb/rpm/AppImage and Windows NSIS setup + portable exe via `cargo-xwin` (MSVC target), built in parallel. Tool versions are pinned by digest/checksum in the Dockerfile (incl. `TAURI_CLI_VERSION`).
- Release builds use `windows_subsystem = "windows"` (`not(debug_assertions)` only): no console output on Windows (`--list`, `--help`, errors are invisible there). Debug builds / `cargo run` keep the console.

## Architecture

Flow: `main.rs` parses CLI and (unless launcher / `--edit`) loads config **before** Tauri starts (all errors surface early, nothing is opened on failure) → `inject::build_scripts` reads every script/style file into init-script strings → `window::open` builds one header + content window per app in `setup`.

- `config.rs`: serde structs with `deny_unknown_fields`; app `name` must match `[A-Za-z0-9_-]+` (it ends up in window labels, the profile dir and the AppUserModelID); `Config::load` = read + `Config::parse_at` (resolves script/style paths relative to the config file). Config lookup: `--config` > `config.toml` next to the executable (`$APPIMAGE` dir for AppImages) > `<config dir>/weboverlay/config.toml`. `allowed_hosts` defaults to the host of `url`; `*.x` also matches bare `x`. `wildcard_match` is the shared `*` matcher (case-insensitive).
- `inject.rs`: each style and each script becomes a **separate** `initialization_script` so a syntax error in one user file doesn't kill the others. Scripts are wrapped in an IIFE + `try/catch`, URL `matches` are compiled to anchored JS regexes, `run_at` maps to immediate / `DOMContentLoaded` / `load`. CSS goes through constructable stylesheets (`adoptedStyleSheets`, bypasses page CSP) with a `<style>` fallback. User content is embedded via `serde_json::to_string` (JSON string = valid JS literal).
- `window.rs`: each app = one undecorated `Window` (label `<name>`, Tauri `unstable` multiwebview) with two children: `<name>:header` (local `dist/header.html`, fixed `HEADER_HEIGHT`) above `<name>:content` (the remote site). Linux ignores child bounds (children are packed in the window's `GtkBox`), so `fix_header_height` makes the header a non-expanding GTK child; elsewhere `relayout` sets bounds on resize. Content: per-app profile via `data_directory` (`<local data dir>/weboverlay/profiles/<name>`, header shares it), `on_navigation` keeps allowed http(s) hosts and internal schemes in-app and sends the rest to the system browser (`tauri_plugin_opener::open_url`, called directly, plugin not registered); `on_new_window`: allowed hosts get a plain decorated `WebviewWindow` popup (`window_features` keeps `window.opener` for OAuth), others open externally. Content and popup builders share the `configure!` macro.
- `header.rs`: per-window state (`States`, keyed by window label: title, URL, zoom) pushed to the header with `header.eval("window.weboverlay.update(...)")` on content page load / title change / resize. `header_action` is the only command; the header calls it via `window.__TAURI_INTERNALS__.invoke` (no npm, no `withGlobalTauri`) and it rejects callers other than `<window>:header`. Window controls are done in Rust, so no `core:window:*` permissions are needed. Favicon: `refresh_favicon` runs `FAVICON_JS` in the content with `eval_with_callback` (on load and title change; no page IPC) and `sanitize_favicon` keeps only http(s) / `data:image/` URLs before they reach the privileged header.
- `icon.rs`: when the favicon changes, a thread downloads it with `ureq` (page/header can't read cross-origin image bytes), falls back to `/favicon.ico` then the app icon, and `window.set_icon`s it (Linux X11 only; Wayland uses the `.desktop` file). Tauri's `set_icon` only sets Win32 `ICON_SMALL`, so on Windows `set_taskbar_icon` also sends `WM_SETICON ICON_BIG` (taskbar / Alt+Tab), resized to `SM_CXICON`, destroying the HICONs it created. The `image` dependency enables more formats in `tauri::image::Image::from_bytes`; SVG favicons (incl. `data:image/svg+xml,` emoji `<text>` icons) are rasterized with `resvg` using system fonts (loaded once), with file `<image href>` disabled. WSL has no emoji font: check emoji rendering by running the cross-compiled test binary on Windows with `WEBOVERLAY_ICON_OUT=<png path>`. `set_app_id` gives each app window (and its popups) its own AppUserModelID `dev.blet.weboverlay.<name>` → separate taskbar buttons on Windows.
- `editor.rs` + `dist/editor.html`: decorated `WebviewWindow` `weboverlay:editor` (the `:` can't clash with an app label), opened by `--edit` (no `Config::load`, so a broken/missing config is editable) or the header `edit` action (`open_later`: built from a worker thread, since a window built inside a sync command stays blank on Windows and `run_on_main_thread` runs inline on the main thread). Commands `editor_files/read/write/restart` reject callers other than the editor webview, and read/write only paths listed by `files()` (config + scripts/styles of the **saved** config). Writing the config runs `Config::parse` first and refuses invalid TOML; CRLF files keep CRLF. Restart checks `Config::load`, spawns `config::exe_path()` with the original args minus `--edit` (`strip_edit_flag`), then exits. Highlighting is a sticky-regex tokenizer in a `<pre>` under a transparent `<textarea>`.
- `launcher.rs` + `dist/launcher.html`: decorated `WebviewWindow` `weboverlay:launcher`, opened when there is no app and no `--config` (`Mode::Launcher`, no `Config::load`). Lists recent apps (`<local data dir>/weboverlay/recent.json`, written by `launcher::record` from `prepare` on every normal launch, dedup on config + app, max 10, paths via `std::path::absolute`) and the apps of `config::portable_path()` (only when present) and `config::user_path()`. `launcher_open/edit` reject callers other than the launcher and only accept configs/apps currently listed, pre-check `Config::load` + `build_scripts` (the child would fail silently), then spawn `exe --config <path> [<app>]` / `--edit --config <path>` (child reaped from a thread); the launcher stays open and refreshes on focus.
- Navigation shortcuts in the content are a plain init script (`inject::SHORTCUTS_SCRIPT`), no IPC.
- Security model: no capabilities file and no `remote.urls`. Tauri allows app commands only to local origins when no app ACL manifest exists, so the remote content gets `"... not allowed"` for `header_action` (verified on Linux and Windows). Adding a capability or `AppManifest` changes this: re-check that remote content stays denied.
- `dist/index.html` is only a placeholder required by `generate_context!`; `dist/header.html` is the header UI (self-contained, light/dark). `tauri.conf.json` declares no windows (all created in code). `gen/` is generated by `tauri-build` (gitignored): don't edit.

## Notes

- Injection is main-frame only (`initialization_script`, not `_for_all_frames`). Scripts run in page context and remain subject to page CSP for `eval` etc.
- Script/style files are read once at startup; changes need an app restart.
- X11 check of the window icon: run with `GDK_BACKEND=x11`, then `xprop -name <page title> _NET_WM_ICON | head -c 60` shows the icon size.
- Back/forward buttons are always enabled (no can-go-back API). SPA URL changes only reach the header on title change.
- Live behaviour can be checked headlessly-ish: serve a local page (`python3 -m http.server`), point an app at it, and have test scripts beacon results back via `new Image().src = "/beacon?..."` read from the server log (e.g. content `innerHeight` = window height − 36 checks the header layout). Header actions can be exercised by temporarily adding `invoke(...)` calls to `dist/header.html`.

## Conventions

- Update `CHANGELOG.md` `[Unreleased]` (Keep a Changelog) and `README.md` for user-visible changes; update `examples/config.toml` and the config section of `README.md` when the config format changes. A release removes the `[Unreleased]` heading: add it back above the latest version (its compare link at the bottom stays).
- Before committing: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`; also `cargo clippy --target x86_64-pc-windows-gnu -- -D warnings` when `cfg(windows)` code changes.
- Commit each finished feature or fix with the `commit-gpg` skill (GPG-signed commit, CHANGELOG checked).
