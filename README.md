# WebOverlay

Open any website as a standalone desktop webapp (Tauri 2) and inject your own JavaScript and CSS into it.

- One window per app, isolated profile (cookies, storage) per app.
- Custom header replacing the OS title bar: back, forward, reload, home, favicon + page title + URL, zoom, edit config, open in browser, window controls.
  Shortcuts: Alt+←/→, F5 / Ctrl+R.
- Site favicon as window/taskbar icon (Windows, Linux X11), one taskbar button per app on Windows; a pinned app relaunches with its config and favicon (icon taken when pinned).
- Scripts and styles injected on every page load, filtered by URL wildcards.
- Built-in editor for the config and its script/style files (TOML/JS/CSS highlighting).
- Off-host links and popups open in the system browser.
- Remote pages get no Tauri IPC access.

## Prerequisites

Release builds and checks: Docker with buildx only (any Linux).

Dev run (`cargo run`) on Linux:

```sh
sudo apt install pkg-config build-essential libssl-dev libwebkit2gtk-4.1-dev \
  libsoup-3.0-dev libjavascriptcoregtk-4.1-dev librsvg2-dev libayatana-appindicator3-dev \
  libdbus-1-dev libxdo-dev
```

Windows: Rust (MSVC toolchain) and WebView2 (preinstalled on Windows 10/11).

## Usage

```sh
cargo run -- --config examples/config.toml demo   # dev
scripts/check.sh                                  # fmt, clippy (Linux + Windows), tests in Docker
scripts/build.sh                                  # release builds in Docker (see below)
weboverlay [--config <path>] [--list] [--edit] [--maximized] [<app>...]  # no app = open all
weboverlay                                        # launcher
```

## Release builds

`scripts/build.sh` (`docker buildx bake`) builds both platforms in parallel from any Linux, only Docker (with buildx) required. `scripts/build.sh [--fast] [linux|windows]...` builds only the given platforms; `--fast` trades a slightly bigger binary for a much quicker rebuild (thin LTO):

- `dist-tauri/linux/`: `.deb`, `.rpm`, `.AppImage`
- `dist-tauri/windows/`: NSIS `*-setup.exe` and `WebOverlay-portable.exe` (cargo-xwin, MSVC target)

Release Windows builds have no console: `--list`, `--help` and errors print nothing there.

Config lookup (first found):

1. `--config <path>`
2. `config.toml` next to the executable (portable mode; next to the `.AppImage` file for AppImages)
3. `<config dir>/weboverlay/config.toml` (`~/.config/weboverlay/config.toml`, `%APPDATA%\weboverlay\config.toml`)

Profiles: `<local data dir>/weboverlay/profiles/<app>`.

## Config

```toml
[[app]]
name = "gmail"                     # [A-Za-z0-9_-]+, used on the command line
url = "https://mail.google.com"
title = "Gmail"                    # optional, default: page title
width = 1200                       # optional
height = 800                       # optional
maximized = false                  # optional, start maximized (also `--maximized` on the command line)
user_agent = "..."                 # optional
allowed_hosts = ["*.google.com"]   # optional, default: host of url; `*.x` also matches `x`
styles = [{ path = "gmail/dark.css", matches = [] }]
scripts = [
  { path = "gmail/main.js", run_at = "document-end", matches = ["https://mail.google.com/mail/*"] },
]
```

| Field     | Values |
| --------- | ------ |
| `run_at`  | `document-start` (before page scripts), `document-end` (DOMContentLoaded, default), `document-idle` (load) |
| `matches` | Wildcards (`*`) on `location.href`, case-insensitive; empty = every page |

Paths are relative to the config file. Files are read at startup: restart the app after editing.

## Launcher

`weboverlay` without app nor `--config` opens the launcher instead of the apps:

- **Recent**: the last 10 apps opened (from the launcher or the command line), stored in
  `<local data dir>/weboverlay/recent.json`; apps no longer in their config are greyed out.
- **Folder config** (`config.toml` next to the executable, when present) and **User config**
  (`<config dir>/weboverlay/config.toml`): their apps, `Open all`, and `Edit` (`Create` when missing).
- **Config**: configs created with `New config…` (native "save as" dialog; the file is created from a
  commented example of every key unless it exists, then opened in the editor), stored in
  `<local data dir>/weboverlay/configs.json`; `Forget` removes one from the list (the file is kept).

One click starts the app in a new WebOverlay process (`--config <path> <app>`); the launcher stays open.
Cards show the app favicon: the last one seen by the app (cached as
`<local data dir>/weboverlay/icons/<id>.png`), else the site's `/favicon.ico`, else a letter.

## Editor

The pencil button in the header (or `weboverlay --edit`, which opens only the editor and works with a
missing or invalid config) edits `config.toml` and the script/style files it references:

- TOML/JS/CSS highlighting, line numbers, Tab / Shift+Tab indent, Ctrl+S save.
- `config.toml` is validated before saving: an invalid config is refused with the error shown.
- Files referenced by the config but missing are marked `new` and created on save.
- Restart relaunches WebOverlay (without `--edit`) to apply the changes.

Each script runs inside its own function wrapped in `try/catch`; errors are logged to the devtools console
(right click → Inspect in debug builds).

## Limitations

- Injection targets the main frame only (no iframes).
- Styles use constructable stylesheets (bypass CSP), falling back to a `<style>` element.
- Scripts run in the page context and are subject to the page CSP for things like `eval`.
