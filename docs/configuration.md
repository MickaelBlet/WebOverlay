# Configuration

One config file = one app window (`[app]` table; a second app goes in another config file).
The [editor](editor.md) completes and documents every key while typing.

```toml
[app]
name = "gmail"                     # [A-Za-z0-9_-]+, names the profile (configs with the same name share it)
url = "https://mail.google.com"
title = "Gmail"                    # optional, default: page title
width = 1200                       # optional
height = 800                       # optional
maximized = false                  # optional, start maximized (also `--maximized` on the command line)
center = false                     # optional, open the window centered on the screen
zoom = 1.25                        # optional, initial page zoom (0.3 to 3.0, default 1.0), also the zoom reset
user_agent = "..."                 # optional
icon = "gmail/icon.png"            # optional, header/window/taskbar/launcher icon (PNG, ICO, JPEG, GIF, WebP, BMP, SVG), default: site favicon
allowed_hosts = ["*.google.com"]   # optional, default: host of url; `*.x` also matches `x`
open_downloads = ["ica"]           # optional, file extensions opened with their default app once downloaded (no save dialog)
styles = [{ path = "gmail/dark.css", matches = [] }]
scripts = [
  { path = "gmail/main.js", run_at = "document-end", matches = ["https://mail.google.com/mail/*"] },
]

[app.header]                       # optional
color = "#c5221f"                  # optional, background (#rgb / #rrggbb), default: system theme
text_color = "#ffffff"             # optional, default: light or dark, following `color`
hide = ["zoom", "edit"]            # optional, header items not shown

[app.vault]                        # optional
fill = "button"                    # optional, `button` (default) or `auto`
```

| Field     | Values |
| --------- | ------ |
| `run_at`  | `document-start` (before page scripts), `document-end` (DOMContentLoaded, default), `document-idle` (load) |
| `matches` | Wildcards (`*`) on `location.href`, case-insensitive; empty = every page |
| `fill`    | `button`: header vault button only (default); `auto`: also on page load when exactly one login is saved for the page origin |
| `hide`    | `back`, `forward`, `reload`, `home`, `favicon`, `title`, `url`, `zoom` (the 3 zoom buttons), `downloads`, `vault`, `edit`, `devtools`, `external` (window controls always stay) |

Paths are relative to the config file. Files are read at startup: restart the app after editing
(the editor's Restart button does it).

A complete example with a script and a style: [`examples/config.toml`](../examples/config.toml).

## Scripts and styles

- Injected on every page load of the main frame whose URL matches `matches`.
- Each script runs inside its own function wrapped in `try/catch`: an error in one file doesn't stop
  the others. Errors are logged to the devtools console (header terminal button, `F12` /
  `Ctrl+Shift+I` with the header focused, or right click → Inspect).
- Styles use constructable stylesheets (they bypass the page CSP), falling back to a `<style>` element.

## Limitations

- Injection targets the main frame only (no iframes).
- Scripts run in the page context and are subject to the page CSP for things like `eval`.
