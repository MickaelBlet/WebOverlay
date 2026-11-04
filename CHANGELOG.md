# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Linux: `--password-store=basic` (as in Chromium) keeps the vault passwords in a plaintext file readable by the user only, for systems without a Secret Service; its logins are separate from the keyring ones. The launcher forwards it to the apps and editors it opens.

## [0.2.5] - 2026-11-04

### Added

- Vault: logins (site, username, password) saved in the system keyring (Secret Service on Linux, Credential Manager on Windows) and managed in a vault window. The header key button fills the login saved for the page into its login form, or opens the vault; `[app.vault] fill = "auto"` also fills it on page load. `vault` header item for `hide`. The config editor opens the vault too (key button, `WebOverlay: Manage Logins (Vault)` command), also without the app running.

## [0.2.4] - 2026-11-02

### Fixed

- Linux: app windows, the launcher and the editor can be resized from their borders and corners (Tauri's undecorated window resize handles are not attached to webviews created with its multiwebview `unstable` feature).
- Linux: much faster rendering without a usable GPU (virtual machine without 3D acceleration such as VMware, no GPU render node): WebKitGTK's accelerated compositing (and DMA-BUF renderer) is turned off and Mesa uses its software renderer directly, which also silences the `VMware: No 3D enabled` / `failed to create dri2 screen` messages. `WEBOVERLAY_GPU=1` keeps the GPU path, `WEBOVERLAY_GPU=0` forces the software one; `WEBKIT_*` / `LIBGL_ALWAYS_SOFTWARE` variables already set are kept.
- AppImage: no more `Failed to load module "canberra-gtk-module"` / `"pk-gtk-module"` messages (the host's `GTK_MODULES` cannot be loaded by the bundled GTK and is ignored).

## [0.2.3] - 2026-11-01

### Added

- Config editor: a Pin button (and the `View: Toggle Always on Top` command) keeps the editor window above the other windows.
- Config editor: new scripts and styles from the sidebar (`Ctrl+N`; an existing file is refused) or imported (copied next to the config), added to `config.toml` with its comments and layout kept; rename, remove from the config, delete, reveal in the file manager and copy path from the file context menu.
- Config editor: tabs, a second editor group side by side (`Ctrl+\`), a resizable and collapsible sidebar (`Ctrl+B`); open tabs and layout are restored per config.
- Config editor: rename the config file from its context menu (same directory); the launcher's recent and added configs follow.
- Config editor: closing the window or restarting with unsaved changes asks to save them.
- `center` app key: opens the app window centered on the screen (default `false`).
- Launcher: the window opens centered on the screen.
- Launcher: lists the apps of every `*.toml` file of the user config directory (`~/.config/weboverlay/`, `%APPDATA%\weboverlay\`) and next to the executable (next to the `.AppImage` file for AppImages), not only `config.toml`.
- Launcher: an import button at the bottom right (and `Import…` in the empty state) adds an existing config file to the launcher with a native open dialog (the file stays in place).

### Changed

- Launcher: the `Configs` button arrow is an SVG chevron (was a `▾` character) and points up while the menu is open.
- Launcher: `Create` / `Create user config` writes the missing config from the commented example of every key (like `New config…`) before opening the editor, instead of an empty file.
- Launcher: `New config…` moved from the title bar to a `+` button at the bottom right (still in the empty state).
- **Breaking**: one app per config file. The config holds a single `[app]` table instead of the `[[app]]` list (an `[[app]]` config is refused with an explicit error: move each app into its own config file). The command line no longer takes app names (`weboverlay --config <path>` opens its app), the launcher's config menu shows the app with `Open` instead of `Open all`, and a pinned Windows taskbar app relaunches with `--config <path>` only (re-pin apps pinned with an older version).
- Launcher: default window width 570 px (was 600).
- Config editor: own title bar (app icon and name, search box for quick open and commands, Pin / Restart / Save) instead of the system one.
- Config editor: Restart reopens the app windows in place and keeps the editor open (it used to relaunch WebOverlay and close the editor).
- The Linux AppImage is self-contained (WebKitGTK, GTK, Mesa and glibc bundled, built with quick-sharun on Arch Linux): it runs on distros without `webkit2gtk-4.1` or with an older glibc, such as RHEL/Rocky 8 (glibc 2.28).

### Fixed

- Launcher: a double click on an app (or on Edit config) no longer starts two instances.
- Launcher: clicking an app or a config while the window is not focused acts at once (the click that focused the window used to be lost when the list refreshed).
- Linux release builds no longer copy the packages of older versions left in the build cache into `dist-tauri/linux/`.

## [0.2.2] - 2026-10-30

### Added

- Config editor: completion suggests common launch-file extensions (`ica`, `rdp`, `jnlp`, `vnc`, `pdf`) for `open_downloads`.

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

[Unreleased]: https://github.com/MickaelBlet/WebOverlay/compare/v0.2.5...HEAD
[0.2.5]: https://github.com/MickaelBlet/WebOverlay/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/MickaelBlet/WebOverlay/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/MickaelBlet/WebOverlay/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/MickaelBlet/WebOverlay/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/MickaelBlet/WebOverlay/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/MickaelBlet/WebOverlay/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/MickaelBlet/WebOverlay/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/MickaelBlet/WebOverlay/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MickaelBlet/WebOverlay/releases/tag/v0.1.0
