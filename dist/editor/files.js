// Sidebar (config, scripts, styles), file operations (new / rename / remove / delete, each one
// also editing `config.toml` on the Rust side) and the file context menu.

import {
  $, invoke, state, pane, langOf, fileOf, isDirty, relative, relDir, baseName, setMessage, on, el, load, store, modal,
  showMenu, copyText,
} from "./core.js";
import { register, keysOf, runCommand } from "./keymap.js";
import { openFile, refreshFiles, reloadConfig, saveFile, dropPath, renamePath } from "./main.js";

const FT = { toml: "TOML", js: "JS", css: "CSS" };
const TAB_TYPE = "application/x-weboverlay-tab";

/** File type glyph of `path`. */
export const ftIcon = (path) => {
  const lang = langOf(path);
  return el("span", { className: "ft ft-" + (lang || "txt"), textContent: lang === "toml" ? "{}" : FT[lang] || "··" });
};

const PLUS_SVG = '<svg class="i" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>';

export function renderSidebar() {
  const list = $("file-list");
  const open = new Set(state.panes.flatMap((p) => p.tabs));
  const item = (file) =>
    el(
      "div",
      {
        className:
          "file" + (file.path === state.current ? " current" : "") + (open.has(file.path) ? " open" : "") + (isDirty(file.path) ? " dirty" : ""),
        title: file.path,
        draggable: true,
        dataset: { path: file.path },
      },
      ftIcon(file.path),
      el("span", { className: "name", textContent: baseName(file.path) }),
      file.kind !== "config" && relDir(file.path) ? el("span", { className: "dir", textContent: relDir(file.path) }) : null,
      !(state.buffers.get(file.path)?.exists ?? file.exists)
        ? el("span", { className: "badge", textContent: "missing", title: "Not created yet: saved on the first Save" })
        : null,
      el("span", { className: "dot" }),
    );
  const group = (title, kind) => {
    const files = state.files.filter((f) => f.kind === kind);
    const head = el(
      "div",
      { className: "group-title" },
      el("span", { textContent: title }),
      kind !== "config" ? el("span", { className: "count", textContent: files.length || "" }) : null,
    );
    const nodes = [head, ...files.map(item)];
    if (kind !== "config" && !state.configError) {
      head.append(
        el("button", {
          className: "icon-btn add",
          title: `New ${kind === "script" ? "Script" : "Style"}…`,
          tabIndex: -1,
          innerHTML: PLUS_SVG,
          onclick: () => newFile(kind),
        }),
      );
      if (!files.length) {
        nodes.push(
          el(
            "div",
            { className: "side-empty" },
            `No ${title.toLowerCase()} yet. `,
            el("a", { textContent: "Add one", onclick: () => newFile(kind) }),
          ),
        );
      }
    }
    return nodes;
  };
  list.replaceChildren(
    ...group("Config", "config"),
    ...(state.configError
      ? [el("div", { className: "side-empty", textContent: "Fix and save the config to list its scripts and styles." })]
      : [...group("Scripts", "script"), ...group("Styles", "style")]),
  );
}

$("file-list").addEventListener("click", (e) => {
  const item = e.target.closest(".file");
  if (item) openFile(item.dataset.path);
});
$("file-list").addEventListener("auxclick", (e) => {
  // Middle click: open in the other group.
  const item = e.target.closest(".file");
  if (item && e.button === 1) openInOther(item.dataset.path);
});
$("file-list").addEventListener("contextmenu", (e) => {
  const item = e.target.closest(".file");
  e.preventDefault();
  if (!item) {
    if (!state.configError) {
      showMenu(e.clientX, e.clientY, [
        { label: "New Script…", run: () => newFile("script") },
        { label: "New Style…", run: () => newFile("style") },
        "-",
        { label: "Import Script or Style…", run: importFile },
      ]);
    }
    return;
  }
  fileMenu(e.clientX, e.clientY, item.dataset.path, [
    { label: "Open", run: () => openFile(item.dataset.path) },
    { label: "Open to the Side", run: () => openInOther(item.dataset.path) },
  ]);
});
$("file-list").addEventListener("dragstart", (e) => {
  const item = e.target.closest(".file");
  if (!item) return;
  e.dataTransfer.setData(TAB_TYPE, JSON.stringify({ path: item.dataset.path, from: -1 }));
  e.dataTransfer.effectAllowed = "copyMove";
});

function openInOther(path) {
  const target = state.panes.find((p) => p !== pane);
  if (target) openFile(path, target);
  else openFile(path).then(() => runCommand("split"));
}

/** Context menu of `path` (sidebar item or tab), after the `first` entries. */
export function fileMenu(x, y, path, first = []) {
  const file = fileOf(path);
  const entry = file && file.kind !== "config";
  showMenu(x, y, [
    ...first,
    ...(first.length ? ["-"] : []),
    { label: "Rename…", disabled: !file, run: () => renameFile(path) },
    { label: "Remove from Config", disabled: !entry, run: () => unreferenceFile(path) },
    { label: "Delete File", danger: true, disabled: !entry, run: () => deleteFile(path) },
    "-",
    { label: "Reveal in File Manager", disabled: !file, run: () => reveal(path) },
    { label: "Copy Path", run: () => copyText(path) },
    { label: "Copy Relative Path", run: () => copyText(relative(path)) },
  ]);
}

// ---- Operations ----

/** File operations rewrite the saved config: it must not have unsaved changes. */
async function configReady() {
  const config = state.configPath;
  if (!isDirty(config)) return true;
  const { button } = await modal({
    title: "Save the config first?",
    body: `${baseName(config)} has unsaved changes. This operation edits it on disk.`,
    buttons: [
      { id: "cancel", label: "Cancel" },
      { id: "save", label: "Save", primary: true },
    ],
  });
  return button === "save" && saveFile(config);
}

async function after(message) {
  await refreshFiles();
  await reloadConfig();
  setMessage(message + " Restart to apply.", "ok");
}

const RUN_AT = [
  ["document-end", "document-end (DOM ready, default)"],
  ["document-start", "document-start (before the page scripts)"],
  ["document-idle", "document-idle (after load)"],
];
const kindOfPath = (path) => (/\.css$/i.test(path) ? "style" : /\.[mc]?js$/i.test(path) ? "script" : null);

export async function newFile(kind = "script") {
  if (state.configError) return setMessage("Fix and save the config first.", "error");
  if (!(await configReady())) return;
  const dir = state.files.find((f) => f.kind === kind && relDir(f.path))?.path;
  const prefix = dir ? relDir(dir).replace(/\\/g, "/") + "/" : "";
  await entryDialog({ kind, path: prefix, selection: [prefix.length, prefix.length] });
}

/** Adds an existing script or style, picked with the native dialog (copied next to the config). */
export async function importFile() {
  if (state.configError) return setMessage("Fix and save the config first.", "error");
  if (!(await configReady())) return;
  let picked;
  try {
    picked = await invoke("editor_pick_import");
  } catch (e) {
    return setMessage(String(e), "error");
  }
  if (!picked) return;
  const name = picked.path.lastIndexOf("/") + 1;
  await entryDialog({
    kind: kindOfPath(picked.path) || "script",
    path: picked.path,
    selection: [name, picked.path.length],
    source: picked.source,
  });
}

/** The New / Import dialog, then `editor_add`. */
async function entryDialog({ kind, path, selection, source }) {
  const { button, values } = await modal({
    title: source ? `Import ${baseName(source)}` : "New Script or Style",
    body: source
      ? "The file is copied next to the config (kept in place when already there) and added to it."
      : "The file is created next to the config and added to it.",
    fields: [
      { id: "path", label: "Path", value: path, placeholder: kind === "style" ? "style.css" : "main.js", hint: "Relative to the config directory", selection },
      { id: "kind", label: "Type", value: kind, options: [["script", "Script (JavaScript)"], ["style", "Style (CSS)"]] },
      { id: "run_at", label: "Run at", value: "document-end", options: RUN_AT },
      { id: "matches", label: "Matches", placeholder: "*://example.com/*", hint: "URL wildcards, comma-separated; empty = every page" },
    ],
    buttons: [
      { id: "cancel", label: "Cancel" },
      { id: "create", label: source ? "Import" : "Create", primary: true },
    ],
    onInput: (v, inputs, changed) => {
      // The extension picks the type while typing the path; the Type select stays free.
      const k = changed === "path" && kindOfPath(v.path);
      if (k && k !== v.kind) inputs.kind.value = k;
      inputs.run_at.disabled = inputs.kind.value !== "script";
    },
    validate: (v) => {
      const path = v.path.trim();
      if (!path || path.endsWith("/")) return "Enter a file name.";
      if (kindOfPath(path) !== v.kind) return v.kind === "style" ? "A style file name ends with .css." : "A script file name ends with .js.";
      if (state.files.some((f) => relative(f.path).replace(/\\/g, "/") === path.replace(/\\/g, "/"))) return "This file is already in the config.";
      return "";
    },
  });
  if (button !== "create") return;
  try {
    const full = await invoke("editor_add", {
      kind: values.kind,
      path: values.path.trim(),
      runAt: values.kind === "script" ? values.run_at : null,
      matches: values.matches.split(",").map((m) => m.trim()).filter(Boolean),
      import: !!source,
    });
    await after(`${source ? "Imported" : "Added"} ${values.path.trim()}.`);
    await openFile(full);
  } catch (e) {
    setMessage(String(e), "error");
  }
}

async function renameFile(path) {
  if (fileOf(path)?.kind === "config") return renameConfig(path);
  if (!(await configReady())) return;
  if (isDirty(path) && !(await saveFile(path))) return;
  const kind = fileOf(path)?.kind;
  const rel = relative(path).replace(/\\/g, "/");
  const { button, values } = await modal({
    title: `Rename ${baseName(path)}`,
    fields: [{ id: "path", label: "New path", value: rel, hint: "Relative to the config directory", selection: [rel.lastIndexOf("/") + 1, rel.lastIndexOf(".")] }],
    buttons: [
      { id: "cancel", label: "Cancel" },
      { id: "rename", label: "Rename", primary: true },
    ],
    validate: (v) => (kindOfPath(v.path.trim()) === kind ? "" : kind === "style" ? "A style file name ends with .css." : "A script file name ends with .js."),
  });
  if (button !== "rename") return;
  try {
    const to = await invoke("editor_rename", { path, newPath: values.path.trim() });
    renamePath(path, to);
    await after(`Renamed to ${values.path.trim()}.`);
  } catch (e) {
    setMessage(String(e), "error");
  }
}

/** Renames the config in its directory (the script/style paths stay relative to it). */
async function renameConfig(path) {
  if (!(await configReady())) return;
  const name = baseName(path);
  const { button, values } = await modal({
    title: `Rename ${name}`,
    fields: [{ id: "name", label: "New name", value: name, hint: "Same directory. A pinned taskbar button still points to the old file.", selection: [0, name.lastIndexOf(".")] }],
    buttons: [
      { id: "cancel", label: "Cancel" },
      { id: "rename", label: "Rename", primary: true },
    ],
    validate: (v) => {
      const n = v.name.trim();
      if (/[\\/]/.test(n)) return "Enter a file name, not a path.";
      return /^[^.].*\.toml$/i.test(n) ? "" : "A config file name ends with .toml.";
    },
  });
  if (button !== "rename") return;
  try {
    const to = await invoke("editor_rename_config", { name: values.name.trim() });
    if (to === path) return;
    state.configPath = to; // `renamePath` then saves the session (per config path) under it.
    renamePath(path, to);
    await after(`Renamed to ${baseName(to)}.`);
  } catch (e) {
    setMessage(String(e), "error");
  }
}

async function unreferenceFile(path) {
  if (!(await configReady())) return;
  if (isDirty(path) && !(await saveFile(path))) return;
  try {
    await invoke("editor_unreference", { path });
    dropPath(path);
    await after(`Removed ${baseName(path)} from the config (the file is kept).`);
  } catch (e) {
    setMessage(String(e), "error");
  }
}

async function deleteFile(path) {
  if (!(await configReady())) return;
  const { button } = await modal({
    title: `Delete ${baseName(path)}?`,
    body: `${path} is deleted and removed from the config. This cannot be undone.`,
    buttons: [
      { id: "cancel", label: "Cancel" },
      { id: "delete", label: "Delete", danger: true },
    ],
  });
  if (button !== "delete") return;
  try {
    await invoke("editor_delete", { path });
    dropPath(path);
    await after(`Deleted ${baseName(path)}.`);
  } catch (e) {
    setMessage(String(e), "error");
  }
}

function reveal(path) {
  invoke("editor_reveal", { path }).catch((e) => setMessage(String(e), "error"));
}

// ---- Sidebar layout ----

const LAYOUT = "weboverlay.editor.layout";
const layout = load(LAYOUT, {});

function applyLayout() {
  $("sidebar").hidden = !!layout.hidden;
  if (layout.width) $("sidebar").style.setProperty("--side-w", `${layout.width}px`);
}

function toggleSidebar() {
  layout.hidden = !layout.hidden;
  applyLayout();
  store(LAYOUT, layout);
}

$("side-sash").addEventListener("pointerdown", (e) => {
  const sash = e.currentTarget;
  sash.setPointerCapture(e.pointerId);
  sash.classList.add("drag");
  const move = (ev) => {
    layout.width = Math.round(Math.max(160, Math.min(innerWidth * 0.5, ev.clientX)));
    applyLayout();
  };
  sash.addEventListener("pointermove", move);
  sash.addEventListener(
    "pointerup",
    () => {
      sash.classList.remove("drag");
      sash.removeEventListener("pointermove", move);
      store(LAYOUT, layout);
    },
    { once: true },
  );
});
$("side-sash").addEventListener("dblclick", () => {
  delete layout.width;
  $("sidebar").style.removeProperty("--side-w");
  store(LAYOUT, layout);
});
$("add-file").addEventListener("click", () => newFile());
$("import-file").addEventListener("click", importFile);
$("reveal-config").addEventListener("click", () => state.configPath && reveal(state.configPath));
$("hide-sidebar").addEventListener("click", toggleSidebar);
applyLayout();

on("newFile", () => newFile());

const current = (fn) => () => state.current && fileOf(state.current) && fn(state.current);
register([
  { id: "newFile", label: "File: New Script or Style…", keys: ["ctrl+n"], run: () => newFile() },
  { id: "newScript", label: "File: New Script…", run: () => newFile("script") },
  { id: "newStyle", label: "File: New Style…", run: () => newFile("style") },
  { id: "importFile", label: "File: Import Script or Style…", run: importFile },
  { id: "renameFile", label: "File: Rename…", run: current(renameFile) },
  { id: "unreferenceFile", label: "File: Remove from Config", run: current(unreferenceFile) },
  { id: "deleteFile", label: "File: Delete", run: current(deleteFile) },
  { id: "revealFile", label: "File: Reveal in File Manager", keys: ["ctrl+k r"], run: current(reveal) },
  { id: "copyPath", label: "File: Copy Path of Active File", keys: ["ctrl+k p"], run: current(copyText) },
  { id: "copyRelativePath", label: "File: Copy Relative Path of Active File", run: current((p) => copyText(relative(p))) },
  { id: "toggleSidebar", label: "View: Toggle Sidebar Visibility", keys: ["ctrl+b"], run: toggleSidebar },
]);
$("add-file").title = `New Script or Style… (${keysOf("newFile")})`;
