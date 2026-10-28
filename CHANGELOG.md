# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Open websites listed in a TOML config as standalone webapp windows (Tauri 2, Linux and Windows).
- Isolated profile (cookies, storage) per app.
- Custom header replacing the OS title bar: back, forward, reload, home, favicon, page title, URL, zoom, open in browser, window controls.
- Navigation shortcuts: Alt+←/→, F5 / Ctrl+R.
- Site favicon as window/taskbar icon (Windows, Linux X11), SVG favicons rasterized; one taskbar button per app on Windows.
- User JavaScript and CSS injected on every page load, filtered by URL wildcards, with `run_at` timing.
- Off-host links and popups open in the system browser; allowed-host popups keep `window.opener` (OAuth).
- Remote pages get no Tauri IPC access.
- Start maximized: `maximized = true` per app or `--maximized` on the command line.
- Built-in config editor (header pencil button or `--edit`): `config.toml` and its script/style files, TOML/JS/CSS highlighting, config validated on save, Restart button.
- Launcher when started without app nor `--config`: recent apps and the apps of the user and portable configs, opened in one click.
- Launcher `New config…` button: creates a config file anywhere with a native save dialog (pre-filled with a commented example of every key), lists it in the launcher and opens it in the editor.
- Docker release builds (`scripts/build.sh [--fast] [linux|windows]...`): Linux `.deb`/`.rpm`/`.AppImage`, Windows NSIS setup and portable exe; one platform at a time, `--fast` for quicker thin-LTO rebuilds.
