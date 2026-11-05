# Getting started

## Install

Release packages (see [Building](building.md) to make them):

| File | Platform |
| ---- | -------- |
| `.deb`, `.rpm` | Linux with glibc 2.35+ and `webkit2gtk-4.1` (Ubuntu 22.04, Debian 12…) |
| `.AppImage` | Any Linux, self-contained (bundles WebKitGTK, GTK and glibc), runs on older distros such as RHEL/Rocky 8 |
| `*-setup.exe` | Windows installer (NSIS) |
| `WebOverlay-portable.exe` | Windows, no install |

Windows needs WebView2, preinstalled on Windows 10/11.

## First launch

`weboverlay` without arguments opens the [launcher](launcher.md). Click `+` (New config…) to create a
config from a commented example: it opens in the [editor](editor.md). Set `name` and `url`, save, then
Restart (or open the app from the launcher).

A minimal config:

```toml
[app]
name = "board"
url = "https://board.example.com"
```

See [Configuration](configuration.md) for every key.

## Command line

```sh
weboverlay                                  # launcher
weboverlay --config <path>                  # open the app of this config
weboverlay --config <path> --edit           # open only the editor (works with a missing or invalid config)
weboverlay --list                           # print the app of the config
weboverlay --maximized                      # start maximized
weboverlay --password-store=basic           # Linux: vault without a Secret Service (see Vault)
```

Short forms: `-c`, `-e`, `-l`, `-m`, `-h`. Release Windows builds have no console: `--list`, `--help`
and errors print nothing there.

## Where things live

Config lookup (first found):

1. `--config <path>`
2. `config.toml` next to the executable (portable mode; next to the `.AppImage` file for AppImages)
3. `<config dir>/weboverlay/config.toml` (`~/.config/weboverlay/config.toml`, `%APPDATA%\weboverlay\config.toml`)

Under `<local data dir>/weboverlay/` (`~/.local/share/weboverlay`, `%LOCALAPPDATA%\weboverlay`):

| Path | Contents |
| ---- | -------- |
| `profiles/<app name>` | Browser profile of the app (cookies, storage); configs with the same `name` share it |
| `recent.json` | Recently opened apps (launcher) |
| `configs.json` | Configs created or imported in the launcher |
| `icons/` | Cached favicons (launcher tiles, Windows pinned icons) |
| `vault/` | Login lists of the [vault](vault.md) (no passwords, except with `--password-store=basic`) |
