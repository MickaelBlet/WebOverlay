// Editor entry point: files, buffers, save/restart and the wiring of the other modules.

import {
  $, text, invoke, state, langOf, currentLang, LANG_NAMES, fileOf, relative, baseName, setMessage, emit,
} from "./core.js";
import { highlight } from "./highlight.js";
import * as view from "./view.js";
import * as edit from "./edit.js";
import { register, handleKey } from "./keymap.js";

let restartArmed = false;
/** Paths, most recently used first (Ctrl+Tab). */
const mru = [];

const isDirty = (path) => {
  const b = state.buffers.get(path);
  return !!b && b.content !== b.saved;
};

function renderSidebar() {
  const nav = $("sidebar");
  nav.textContent = "";
  let group = null;
  for (const file of state.files) {
    const title = file.kind === "config" ? "Config" : file.app;
    if (title !== group) {
      group = title;
      const h = document.createElement("div");
      h.className = "group-title";
      h.textContent = title;
      nav.append(h);
    }
    const item = document.createElement("div");
    item.className = "file";
    item.classList.toggle("current", file.path === state.current);
    item.classList.toggle("dirty", isDirty(file.path));
    item.title = file.path;
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = file.kind === "config" ? baseName(file.path) : relative(file.path);
    item.append(name);
    const buffer = state.buffers.get(file.path);
    if (!(buffer ? buffer.exists : file.exists)) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = "new";
      item.append(badge);
    }
    const dot = document.createElement("span");
    dot.className = "dot";
    item.append(dot);
    item.addEventListener("click", () => openFile(file.path));
    nav.append(item);
  }
}

let renderQueued = false;
function renderCode() {
  renderQueued = false;
  // A trailing newline would otherwise collapse the last (empty) line of the overlay.
  $("hl-code").innerHTML = highlight(text.value, currentLang()) + "\n ";
  view.renderGutter();
  view.render();
}
const queueRender = () => {
  if (!renderQueued) {
    renderQueued = true;
    requestAnimationFrame(renderCode);
  }
};

/** Caret moved: position, current line, brackets. */
function caretMoved() {
  const v = text.value;
  const { line, col } = view.lineCol(text.selectionStart, v);
  const sel = text.selectionEnd - text.selectionStart;
  $("pos").textContent = state.current ? `Ln ${line + 1}, Col ${col + 1}` + (sel ? ` (${sel} selected)` : "") : "";
  view.updateBrackets();
  view.renderGutter();
  view.scrolled();
  emit("caret");
}

function updateChrome() {
  const dirty = state.current && isDirty(state.current);
  $("save").disabled = !dirty;
  $("path").textContent = state.current ? "‎" + state.current : "";
  $("lang").textContent = state.current ? LANG_NAMES[langOf(state.current)] || "Plain text" : "";
  document.title = (dirty ? "• " : "") + (state.current ? baseName(state.current) + " — " : "") + "WebOverlay editor";
  for (const item of $("sidebar").querySelectorAll(".file")) {
    item.classList.toggle("current", item.title === state.current);
    item.classList.toggle("dirty", isDirty(item.title));
  }
}

function storeView() {
  const b = state.current && state.buffers.get(state.current);
  if (!b) return;
  Object.assign(b, {
    selStart: text.selectionStart,
    selEnd: text.selectionEnd,
    scrollTop: text.scrollTop,
    scrollLeft: text.scrollLeft,
  });
}

async function openFile(path) {
  if (path === state.current) {
    text.focus();
    return;
  }
  storeView();
  if (!state.buffers.has(path)) {
    try {
      const { content, exists } = await invoke("editor_read", { path });
      state.buffers.set(path, { content, saved: content, exists, selStart: 0, selEnd: 0, scrollTop: 0, scrollLeft: 0 });
    } catch (e) {
      setMessage(String(e), "error");
      return;
    }
  }
  state.current = path;
  mru.splice(0, mru.length, path, ...mru.filter((p) => p !== path));
  const b = state.buffers.get(path);
  text.disabled = false;
  text.value = b.content;
  text.setSelectionRange(b.selStart, b.selEnd);
  emit("open");
  renderCode();
  text.scrollTop = b.scrollTop;
  text.scrollLeft = b.scrollLeft;
  text.focus();
  updateChrome();
  caretMoved();
  if (!b.exists) setMessage("New file: it is created on save.", "warn");
}

async function refreshFiles() {
  const list = await invoke("editor_files");
  state.files = list.files;
  const config = state.files.find((f) => f.kind === "config");
  state.configDir = config ? config.path.replace(/[\\/][^\\/]*$/, "") : "";
  renderSidebar();
  if (list.error) setMessage("Saved config is invalid: " + list.error, "error");
  return list;
}

async function save() {
  const path = state.current;
  if (!path || !isDirty(path)) return;
  const b = state.buffers.get(path);
  const content = b.content;
  try {
    await invoke("editor_write", { path, content });
  } catch (e) {
    setMessage(String(e), "error");
    return;
  }
  b.saved = content;
  b.exists = true;
  setMessage("Saved " + baseName(path) + ". Restart to apply.", "ok");
  if (fileOf(path) && fileOf(path).kind === "config") {
    await refreshFiles().catch((e) => setMessage(String(e), "error"));
  } else {
    renderSidebar();
  }
  updateChrome();
}

async function restart() {
  const dirty = [...state.buffers.keys()].filter(isDirty);
  if (dirty.length && !restartArmed) {
    restartArmed = true;
    setMessage(`Unsaved changes in ${dirty.map(baseName).join(", ")}: click Restart again to discard them.`, "warn");
    return;
  }
  restartArmed = false;
  try {
    await invoke("editor_restart");
  } catch (e) {
    setMessage(String(e), "error");
  }
}

/** Ctrl+Tab: next file in most-recently-used order. */
function cycleFile(dir) {
  const list = mru.filter((p) => fileOf(p)).concat(state.files.map((f) => f.path).filter((p) => !mru.includes(p)));
  if (list.length < 2) return;
  const i = list.indexOf(state.current);
  openFile(list[(i + dir + list.length) % list.length]);
}

// ---- Commands ----

/** Editing commands (only with the text area focused). */
const E = (id, label, keys, run) => ({ id, label, keys, run, editor: true });

register([
  { id: "save", label: "File: Save", keys: ["ctrl+s"], run: save },
  { id: "restart", label: "WebOverlay: Restart", run: restart },
  { id: "nextFile", label: "View: Open Next File", keys: ["ctrl+tab"], run: () => cycleFile(1) },
  { id: "prevFile", label: "View: Open Previous File", keys: ["ctrl+shift+tab"], run: () => cycleFile(-1) },
  E("comment", "Toggle Line Comment", ["ctrl+/"], edit.toggleLineComment),
  E("blockComment", "Toggle Block Comment", ["shift+alt+a"], () => edit.toggleBlockComment()),
  E("moveUp", "Move Line Up", ["alt+arrowup"], () => edit.moveLines(-1)),
  E("moveDown", "Move Line Down", ["alt+arrowdown"], () => edit.moveLines(1)),
  E("copyUp", "Copy Line Up", ["shift+alt+arrowup"], () => edit.copyLines(-1)),
  E("copyDown", "Copy Line Down", ["shift+alt+arrowdown"], () => edit.copyLines(1)),
  E("deleteLine", "Delete Line", ["ctrl+shift+k"], edit.deleteLines),
  E("insertBelow", "Insert Line Below", ["ctrl+enter"], () => edit.insertLine(false)),
  E("insertAbove", "Insert Line Above", ["ctrl+shift+enter"], () => edit.insertLine(true)),
  E("selectLine", "Expand Line Selection", ["ctrl+l"], edit.selectLine),
  E("selectNext", "Select Next Occurrence", ["ctrl+d"], edit.selectNextOccurrence),
  E("indent", "Indent Line", ["ctrl+]"], () => edit.indentLines(false)),
  E("outdent", "Outdent Line", ["ctrl+["], () => edit.indentLines(true)),
  E("redo", "Redo", ["ctrl+y"], () => document.execCommand("redo")),
  E("tab", null, ["tab"], () => edit.tab(false)),
  E("shiftTab", null, ["shift+tab"], () => edit.tab(true)),
  E("enter", null, ["enter"], edit.newline),
  E("home", null, ["home"], () => edit.home(false)),
  E("shiftHome", null, ["shift+home"], () => edit.home(true)),
]);

// ---- Events ----

text.addEventListener("keydown", (e) => {
  if (e.defaultPrevented || e.isComposing) return;
  // On Windows AltGr reports Ctrl+Alt: still a typed char.
  const typed = e.key.length === 1 && !e.metaKey && e.ctrlKey === e.altKey;
  if (typed && edit.typeChar(e.key)) {
    e.preventDefault();
    emit("typed", e.key);
  } else if (e.key === "Backspace" && !e.ctrlKey && !e.altKey && edit.backspacePair()) {
    e.preventDefault();
  }
});

// Capture: commands run before the focused element (text area, find box, palette) sees the key.
window.addEventListener(
  "keydown",
  (e) => {
    if (e.isComposing) return;
    emit("keydown", e);
    if (e.defaultPrevented) return;
    handleKey(e, e.target === text && !text.disabled);
  },
  true,
);

text.addEventListener("input", (e) => {
  const b = state.current && state.buffers.get(state.current);
  if (!b) return;
  const wasDirty = b.content !== b.saved;
  b.content = text.value;
  restartArmed = false;
  if (wasDirty !== isDirty(state.current)) updateChrome();
  queueRender();
  caretMoved();
  emit("change", e);
});
text.addEventListener("scroll", () => {
  view.scrolled();
  emit("scroll");
});
document.addEventListener("selectionchange", () => {
  if (document.activeElement === text) caretMoved();
});
for (const ev of ["copy", "cut", "paste"]) text.addEventListener(ev, edit.clipboard);
new ResizeObserver(() => view.scrolled()).observe(text);

$("save").addEventListener("click", save);
$("restart").addEventListener("click", restart);

document.fonts.ready.then(view.measure);
view.measure();
refreshFiles()
  .then(() => state.files.length && openFile(state.files[0].path))
  .catch((e) => setMessage(String(e), "error"));
