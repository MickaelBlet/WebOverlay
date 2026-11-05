# Troubleshooting

## Linux: slow rendering in a virtual machine

Without a usable GPU (virtual machine without 3D acceleration, no `/dev/dri/renderD*` of a 3D
driver), WebOverlay turns off WebKitGTK's accelerated compositing
(`WEBKIT_DISABLE_COMPOSITING_MODE=1`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`, `LIBGL_ALWAYS_SOFTWARE=1`),
much faster than compositing on Mesa's software renderer.

- `WEBOVERLAY_GPU=1` keeps the GPU path, `WEBOVERLAY_GPU=0` forces the software one.
- Variables already set are kept.

## Page closed or blank on a heavy site

When a page uses too much memory, its web process is stopped (WebKitGTK: by default above the
system RAM, capped at 3 GB; WebView2: when memory runs out) or crashes. WebOverlay reloads the
page; if it crashes again within a minute, the header shows `Page closed: out of memory` (or
`Page crashed`): click it (or `F5`) to reload.

## Page not responding

A page busy in a script (infinite loop, heavy computation) stops reacting to clicks and keys: after
a few seconds the header shows `Page not responding` (removed if the page recovers). Click it (or
`F5` with the header focused) to stop the page and reload it.

## AppImage

The AppImage ignores the host's `GTK_MODULES` (its bundled GTK cannot load them).

## Windows: nothing printed

Release Windows builds have no console: `--list`, `--help` and errors print nothing. Use the
[launcher](launcher.md), which checks configs before starting them and shows their errors, or the
[editor](editor.md), which validates the config while typing.

## Vault: no keyring on Linux

The vault needs a running Secret Service (GNOME Keyring, KWallet, KeePassXC…). Without one, start
with `--password-store=basic` (plaintext file, see [Vault](vault.md)).

## Config changes not applied

Script, style and config files are read at startup: restart the app (editor Restart button).
