# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-10-30

### Added

- Downloads window (header downloads button, shown after the first download; `downloads` value for `[app.header] hide`; opens when a save destination is chosen): progress, speed, cancel, open the file, show it in its folder, clear the list. Replaces the WebView2 download flyout.
- `open_downloads` app key: file extensions (e.g. `["ica"]` for Citrix) whose downloads skip the save dialog, stay in the downloads folder and open with their default app once finished.

### Changed

- Links and popups to hosts outside `allowed_hosts` that point to a download (`Content-Disposition: attachment` or a type browsers don't display, after redirects) are downloaded in the app instead of opening the system browser. The URL is fetched once by the app (no cookies) to decide, so off-host links reach the system browser a little later.
- Linux: the "save as" dialog opens when the download starts instead of once it has finished; cancelling it stops the download.

## [0.2.0] - 2026-10-29

### Added

- `icon` app key: image file (PNG, ICO, JPEG, GIF, WebP, BMP or SVG, relative to the config) used as header, window, taskbar, Windows pin and launcher icon instead of the site favicon; an unreadable or undecodable file stops the launch with an error.
- File downloads from the site and its popups: a native "save as" dialog asks where to save the file (pre-filled with the downloads folder and the file name; cancel discards the download). On Linux the dialog opens once the download has finished.

### Fixed

- Windows: the taskbar button kept an old icon (exe icon, previous favicon or `icon`) for an app that had been pinned once, and a pinned app never picked up a new icon; the pinned shortcuts of the app now follow its current icon.
- Linux: links to files the webview cannot display (`.zip`, `.exe`, PDF...) did nothing; they are downloaded now, and an empty popup opened only for a download (`target="_blank"` link) closes itself.

## [0.1.2] - 2026-10-29

### Added

- Launcher: sort menu at the right of the status bar (name A→Z by default, Z→A, recently opened first), remembered between runs.
- Header: developer tools button (web inspector / console of the site, also F12 / Ctrl+Shift+I), available in release builds; `devtools` value for `[app.header] hide`.

### Fixed

- Launcher: an opened app moves to the top of the recent apps right away (it needed a focus change).

## [0.1.1] - 2026-10-29

### Added

- Launcher shows each app's favicon (the last one seen by the app, else the site's `/favicon.ico`), cached in `<local data dir>/weboverlay/icons/`.
- `zoom` app key: initial page zoom (0.3 to 3.0), also used by the header's zoom reset.
- `[app.header]` config table: header background `color`, `text_color` (default follows the background) and `hide` to remove header items (`back`, `forward`, `reload`, `home`, `favicon`, `title`, `url`, `zoom`, `edit`, `external`).
- Editor: VSCode shortcuts (comment toggle, move/copy/delete line, insert line, select line/next occurrence, indent, smart Home, whole-line copy/cut, redo, Ctrl+Tab file switch), auto-closed brackets and quotes, indentation after an opening bracket, current line and matching bracket highlight.
- Editor: completion with documentation (config keys, values and headers from the config schema; JS keywords, file words and DOM members; CSS properties and values), hover documentation of config keys and values, and live validation of `config.toml` (error underlined, shown in the status bar).
- Editor: find / replace widget (match case, whole word, regex, replace all in one undo step), quick open (`Ctrl+P`), command palette (`Ctrl+Shift+P`, `F1`) and go to line (`Ctrl+G`).
- Launcher: search field (type anywhere to search, `Enter` opens the first match), arrow-key navigation, per-app menu (`⋯` or right click: open, edit config, remove from recent).

### Changed

- New app icon (the launcher logo: two overlapping windows, blue on dark).
- Smaller header bar (30 px instead of 36 px).
- Redesigned launcher: a single grid of app tiles (recent apps first, each app listed once) and a 30 px title bar (undecorated window: drag to move, double click to maximize, own window controls) with search, a `Configs` menu (paths, errors, `Open all` / `Edit` / `Forget`) and `New config…`.
- Smaller launcher window (600×400, minimum 420×240); its title bar shrinks instead of scrolling, so the window controls stay visible.

### Fixed

- Favicon download trusts the OS certificate store, so the window icon works behind corporate TLS inspection (Windows kept the app icon).
- Windows: the launcher and editor show the app icon in the taskbar (the launcher showed an app favicon); the launcher gets its own taskbar button, and its pin reopens the launcher.
- Build: changing a file in `icons/` re-embeds the app icon (cached builds could keep the old exe and window icon).
- No white flash on open: app, launcher and editor windows stay hidden until their local page (header, launcher, editor) has loaded, and the site area of an app window shows the theme background until the site has loaded (Windows flashed white there).

## [0.1.0] - 2026-10-28

### Added

- Open websites listed in a TOML config as standalone webapp windows (Tauri 2, Linux and Windows).
- Isolated profile (cookies, storage) per app.
- Custom header replacing the OS title bar: back, forward, reload, home, favicon, page title, URL, zoom, open in browser, window controls.
- Navigation shortcuts: Alt+←/→, F5 / Ctrl+R.
- Site favicon as window/taskbar icon (Windows, Linux X11), SVG favicons rasterized; one taskbar button per app (and config) on Windows.
- Windows: a pinned app relaunches with its config (`--config <path> <app>`) and keeps its favicon.
- User JavaScript and CSS injected on every page load, filtered by URL wildcards, with `run_at` timing.
- Off-host links and popups open in the system browser; allowed-host popups keep `window.opener` (OAuth).
- Remote pages get no Tauri IPC access.
- Start maximized: `maximized = true` per app or `--maximized` on the command line.
- Built-in config editor (header pencil button or `--edit`): `config.toml` and its script/style files, TOML/JS/CSS highlighting, config validated on save, Restart button.
- Launcher when started without app nor `--config`: recent apps and the apps of the user and portable configs, opened in one click.
- Launcher `New config…` button: creates a config file anywhere with a native save dialog (pre-filled with a commented example of every key), lists it in the launcher and opens it in the editor.
- Docker release builds (`scripts/build.sh [--fast] [linux|windows]...`): Linux `.deb`/`.rpm`/`.AppImage`, Windows NSIS setup and portable exe; one platform at a time, `--fast` for quicker thin-LTO rebuilds.
- `scripts/check.sh`: `cargo fmt --check`, clippy (Linux and Windows) and tests in Docker; no host toolchain or WSL needed.

[Unreleased]: https://github.com/MickaelBlet/WebOverlay/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/MickaelBlet/WebOverlay/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/MickaelBlet/WebOverlay/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/MickaelBlet/WebOverlay/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/MickaelBlet/WebOverlay/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MickaelBlet/WebOverlay/releases/tag/v0.1.0
