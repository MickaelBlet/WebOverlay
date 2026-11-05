# Troubleshooting

## Linux: slow rendering in a virtual machine

Without a usable GPU (virtual machine without 3D acceleration, no `/dev/dri/renderD*` of a 3D
driver), WebOverlay turns off WebKitGTK's accelerated compositing
(`WEBKIT_DISABLE_COMPOSITING_MODE=1`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`, `LIBGL_ALWAYS_SOFTWARE=1`),
much faster than compositing on Mesa's software renderer.

- `WEBOVERLAY_GPU=1` keeps the GPU path, `WEBOVERLAY_GPU=0` forces the software one.
- Variables already set are kept.

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
