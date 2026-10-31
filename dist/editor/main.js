// Editor entry point: buffers, editor groups (tabs, split), save/restart/close and the wiring of
// the other modules.

import {
  $, text, pane, setPane, withPane, onText, attachText, invoke, state, langOf, LANG_NAMES, fileOf, isConfig, isDirty,
  relDir, baseName, setMessage, on, emit, el, load, store, modal,
} from "./core.js";
import { highlight } from "./highlight.js";
import * as view from "./view.js";
import * as edit from "./edit.js";
import { register, handleKey, runCommand } from "./keymap.js";
import { renderSidebar, fileMenu, ftIcon } from "./files.js";
import { setApp } from "./titlebar.js";
import "./complete.js";
import "./check.js";
import "./find.js";
import "./palette.js";

/** Paths, most recently used first (Ctrl+Tab). */
const mru = [];
const X_SVG = '<svg class="i" viewBox="0 0 24 24"><path d="M7 7l10 10M17 7L7 17"/></svg>';
const TAB_TYPE = "application/x-weboverlay-tab";

// ---- Rendering ----

/** Highlight, gutter and decorations of group `p` (default: the active one). */
function renderCode(p = pane) {
  p.renderQueued = false;
  // A trailing newline would otherwise collapse the last (empty) line of the overlay.
  p.hlCode.innerHTML = highlight(p.text.value, langOf(p.current || "")) + "\n ";
  withPane(p, () => {
    view.renderGutter();
    if (p === pane) view.render();
    else view.scrolled();
  });
}
function queueRender(p = pane) {
  if (!p.renderQueued) {
    p.renderQueued = true;
    requestAnimationFrame(() => p.root.isConnected && renderCode(p));
  }
}

/** Caret moved: position, current line, brackets. */
function caretMoved() {
  if (!pane) return;
  const v = text.value;
  const { line, col } = view.lineCol(text.selectionStart, v);
  const sel = text.selectionEnd - text.selectionStart;
  $("pos").textContent = state.current ? `Ln ${line + 1}, Col ${col + 1}` + (sel ? ` (${sel} selected)` : "") : "";
  view.updateBrackets();
  view.renderGutter();
  view.scrolled();
  emit("caret");
}

const sameNames = (p, path) => p.tabs.filter((t) => baseName(t) === baseName(path)).length > 1;

function renderTabs(p) {
  for (const t of p.tabsEl.querySelectorAll(".tab")) t.remove();
  const fill = p.tabsEl.querySelector(".tab-fill");
  for (const path of p.tabs) {
    const buffer = state.buffers.get(path);
    const tab = el(
      "div",
      {
        className: "tab" + (path === p.current ? " current" : "") + (isDirty(path) ? " dirty" : ""),
        title: path,
        draggable: true,
        dataset: { path },
      },
      ftIcon(path),
      el("span", { className: "name", textContent: baseName(path) }),
      sameNames(p, path) && relDir(path) ? el("span", { className: "dir", textContent: relDir(path) }) : null,
      buffer && !buffer.exists ? el("span", { className: "badge", textContent: "new" }) : null,
      !fileOf(path) ? el("span", { className: "badge", textContent: "removed", title: "No longer in the config" }) : null,
      el("button", { className: "x", title: "Close (Ctrl+W)", tabIndex: -1, innerHTML: X_SVG }),
    );
    fill.before(tab);
  }
  p.tabsEl.querySelector(".close-group").hidden = state.panes.length < 2;
  p.tabsEl.querySelector(".tab.current")?.scrollIntoView({ block: "nearest", inline: "nearest" });
}

function updateChrome() {
  const path = state.current;
  const dirty = path && isDirty(path);
  const b = path && state.buffers.get(path);
  $("save").disabled = !dirty;
  $("lang").textContent = path ? LANG_NAMES[langOf(path)] || "Plain text" : "";
  $("saved").textContent = !b ? "" : dirty ? "● Unsaved" : b.exists ? "Saved" : "New file";
  $("saved").classList.toggle("dirty", !!dirty);
  if (!path) $("pos").textContent = "";
  const anyDirty = [...state.buffers.keys()].some(isDirty);
  document.title = (anyDirty ? "• " : "") + (path ? baseName(path) + " — " : "") + "WebOverlay editor";
  renderSidebar();
  for (const p of state.panes) renderTabs(p);
  notifyDirty(anyDirty);
}

let sentDirty = false;
function notifyDirty(dirty) {
  if (dirty === sentDirty) return;
  sentDirty = dirty;
  invoke("editor_dirty", { dirty }).catch(() => {});
}

// ---- Groups ----

function createPane() {
  const root = $("pane-tpl").content.firstElementChild.cloneNode(true);
  const q = (s) => root.querySelector(s);
  const p = {
    root,
    tabsEl: q(".tabs"),
    text: q(".text"),
    hl: q(".hl"),
    hlCode: q(".hl code"),
    gutter: q(".gutter"),
    curline: q(".curline"),
    decoIn: q(".deco-in"),
    wrap: q(".wrap"),
    empty: q(".empty"),
    tabs: [],
    current: null,
    views: new Map(),
    gutterKey: "",
  };
  // Before the `onText` listeners: they run for the active group only.
  p.text.addEventListener("focus", () => activate(p));
  root.addEventListener("mousedown", () => activate(p), true);
  attachText(p);
  new ResizeObserver(() => p.root.isConnected && withPane(p, view.scrolled)).observe(p.text);
  wireTabs(p);
  q(".split").addEventListener("click", () => {
    activate(p);
    splitRight();
  });
  q(".close-group").addEventListener("click", () => closeGroup(p));
  if (state.panes.length) $("groups").append(el("div", { className: "sash", onpointerdown: dragGroups }));
  $("groups").append(root);
  state.panes.push(p);
  return p;
}

function activate(p) {
  if (p === pane) return;
  if (pane) {
    storeView(pane);
    pane.root.classList.remove("active");
  }
  setPane(p);
  p.root.classList.add("active");
  p.wrap.append($("suggest"), $("hover"), $("find"));
  emit("open");
  caretMoved();
  updateChrome();
  saveSession();
}

const other = (p) => state.panes.find((q) => q !== p);

async function splitRight() {
  const from = pane;
  const to = other(from) || createPane();
  if (from.current) await openFile(from.current, to);
  else activate(to);
  layoutGroups();
}

/** Moves the current file of the active group into the other one. */
async function moveToOther() {
  const from = pane;
  const path = from.current;
  if (!path) return;
  const to = other(from) || createPane();
  await openFile(path, to);
  removeTab(from, path);
  layoutGroups();
}

async function closeGroup(p) {
  for (const path of [...p.tabs]) {
    if (!(await closeTab(p, path, { keepPane: true }))) return;
  }
  removePane(p);
}

function removePane(p) {
  if (state.panes.length < 2) return;
  const rest = other(p);
  if (p === pane) activate(rest);
  const sash = p.root.previousElementSibling?.classList.contains("sash") ? p.root.previousElementSibling : p.root.nextElementSibling;
  sash?.remove();
  p.root.remove();
  state.panes.splice(state.panes.indexOf(p), 1);
  for (const path of p.tabs) if (!rest.tabs.includes(path)) state.buffers.delete(path);
  layoutGroups();
  updateChrome();
  saveSession();
}

let groupRatio = 0.5;
function layoutGroups() {
  const [a, b] = state.panes;
  a.root.style.flex = b ? `${groupRatio} 1 0` : "";
  if (b) b.root.style.flex = `${1 - groupRatio} 1 0`;
}

function dragGroups(e) {
  const sash = e.currentTarget;
  sash.setPointerCapture(e.pointerId);
  sash.classList.add("drag");
  const rect = $("groups").getBoundingClientRect();
  const move = (ev) => {
    groupRatio = Math.min(0.85, Math.max(0.15, (ev.clientX - rect.left) / rect.width));
    layoutGroups();
  };
  const up = () => {
    sash.classList.remove("drag");
    sash.removeEventListener("pointermove", move);
    saveSession();
  };
  sash.addEventListener("pointermove", move);
  sash.addEventListener("pointerup", up, { once: true });
}

// ---- Tabs ----

function wireTabs(p) {
  const strip = p.tabsEl;
  const tabOf = (e) => e.target.closest(".tab");
  strip.addEventListener("mousedown", (e) => {
    // Middle click: no autoscroll.
    if (e.button === 1) e.preventDefault();
  });
  strip.addEventListener("auxclick", (e) => {
    const tab = tabOf(e);
    if (e.button === 1 && tab) closeTab(p, tab.dataset.path);
  });
  strip.addEventListener("click", (e) => {
    const tab = tabOf(e);
    if (!tab) return;
    if (e.target.closest(".x")) closeTab(p, tab.dataset.path);
    else openFile(tab.dataset.path, p);
  });
  strip.addEventListener("dblclick", (e) => {
    if (e.target === strip || e.target.classList.contains("tab-fill")) emit("newFile");
  });
  strip.addEventListener("contextmenu", (e) => {
    const tab = tabOf(e);
    if (!tab) return;
    e.preventDefault();
    const path = tab.dataset.path;
    const others = p.tabs.filter((t) => t !== path);
    fileMenu(e.clientX, e.clientY, path, [
      { label: "Close", keys: "Ctrl+W", run: () => closeTab(p, path) },
      { label: "Close Others", disabled: !others.length, run: () => closeMany(p, others) },
      { label: "Close Saved", run: () => closeMany(p, p.tabs.filter((t) => !isDirty(t))) },
      { label: "Close All", run: () => closeMany(p, [...p.tabs]) },
      "-",
      { label: "Split Right", keys: "Ctrl+\\", run: () => openFile(path, p).then(splitRight) },
      { label: other(p) ? "Move into Other Group" : "Move into New Group", run: () => openFile(path, p).then(moveToOther) },
    ]);
  });

  // Drag and drop: reorder, move between groups, drop a sidebar file.
  let drop = null;
  const clear = () => {
    for (const t of document.querySelectorAll(".drop-before, .drop-after, .drop-end")) t.classList.remove("drop-before", "drop-after", "drop-end");
  };
  strip.addEventListener("dragstart", (e) => {
    const tab = tabOf(e);
    if (!tab) return;
    e.dataTransfer.setData(TAB_TYPE, JSON.stringify({ path: tab.dataset.path, from: state.panes.indexOf(p) }));
    e.dataTransfer.effectAllowed = "move";
  });
  strip.addEventListener("dragover", (e) => {
    if (!e.dataTransfer.types.includes(TAB_TYPE)) return;
    e.preventDefault();
    clear();
    const tab = tabOf(e);
    if (tab) {
      const r = tab.getBoundingClientRect();
      const after = e.clientX > r.left + r.width / 2;
      tab.classList.add(after ? "drop-after" : "drop-before");
      drop = p.tabs.indexOf(tab.dataset.path) + (after ? 1 : 0);
    } else {
      strip.classList.add("drop-end");
      drop = p.tabs.length;
    }
  });
  strip.addEventListener("dragleave", (e) => {
    if (!strip.contains(e.relatedTarget)) clear();
  });
  strip.addEventListener("drop", async (e) => {
    clear();
    const data = e.dataTransfer.getData(TAB_TYPE);
    if (!data) return;
    e.preventDefault();
    const { path, from } = JSON.parse(data);
    const source = state.panes[from];
    let at = drop ?? p.tabs.length;
    const i = p.tabs.indexOf(path);
    if (i >= 0) {
      p.tabs.splice(i, 1);
      if (i < at) at--;
    }
    p.tabs.splice(at, 0, path);
    await openFile(path, p);
    if (source && source !== p) removeTab(source, path);
    saveSession();
  });
  strip.addEventListener("dragend", clear);
}

async function closeMany(p, paths) {
  for (const path of paths) if (!(await closeTab(p, path))) return;
}

/** Asks what to do with the unsaved `path`: true when it was saved or may be dropped. */
async function confirmDrop(path) {
  const { button } = await modal({
    title: `Save the changes to ${baseName(path)}?`,
    body: "Your changes will be lost if you don't save them.",
    buttons: [
      { id: "cancel", label: "Cancel" },
      { id: "discard", label: "Don't Save" },
      { id: "save", label: "Save", primary: true },
    ],
  });
  if (button === "save") return saveFile(path);
  return button === "discard";
}

/** Closes the tab `path` of group `p` (asks first when it is the last view of unsaved changes). */
async function closeTab(p, path, { keepPane = false } = {}) {
  const elsewhere = state.panes.some((q) => q !== p && q.tabs.includes(path));
  if (!elsewhere && isDirty(path) && !(await confirmDrop(path))) return false;
  removeTab(p, path, { keepPane });
  if (!elsewhere) state.buffers.delete(path);
  updateChrome();
  return true;
}

/** Removes a tab without asking; shows the next one. */
function removeTab(p, path, { keepPane = false } = {}) {
  const i = p.tabs.indexOf(path);
  if (i < 0) return;
  p.tabs.splice(i, 1);
  p.views.delete(path);
  if (p.current === path) {
    p.current = null;
    const next = mru.find((m) => p.tabs.includes(m)) || p.tabs[Math.min(i, p.tabs.length - 1)];
    if (next) openFile(next, p, { focus: p === pane });
    else show(p);
  }
  if (!p.tabs.length && !keepPane && state.panes.length > 1) removePane(p);
  else renderTabs(p);
  saveSession();
}

/** Closes `path` everywhere without asking (file deleted or no longer editable). */
export function dropPath(path) {
  for (const p of [...state.panes]) removeTab(p, path);
  state.buffers.delete(path);
  mru.splice(0, mru.length, ...mru.filter((m) => m !== path));
  updateChrome();
}

/** `from` was renamed to `to`: buffers, tabs and history follow. */
export function renamePath(from, to) {
  const b = state.buffers.get(from);
  if (b) {
    state.buffers.delete(from);
    state.buffers.set(to, b);
  }
  for (const p of state.panes) {
    p.tabs = p.tabs.map((t) => (t === from ? to : t));
    if (p.views.has(from)) p.views.set(to, p.views.get(from));
    p.views.delete(from);
    if (p.current === from) p.current = to;
  }
  mru.splice(0, mru.length, ...mru.map((m) => (m === from ? to : m)));
  for (const p of state.panes) if (p.current === to) renderCode(p);
  if (state.current === to) emit("open");
  updateChrome();
  saveSession();
}

// ---- Files and buffers ----

function storeView(p) {
  if (!p.current) return;
  p.views.set(p.current, {
    selStart: p.text.selectionStart,
    selEnd: p.text.selectionEnd,
    scrollTop: p.text.scrollTop,
    scrollLeft: p.text.scrollLeft,
  });
}

async function ensureBuffer(path) {
  if (state.buffers.has(path)) return true;
  try {
    const { content, exists } = await invoke("editor_read", { path });
    state.buffers.set(path, { content, saved: content, exists });
    return true;
  } catch (e) {
    setMessage(String(e), "error");
    return false;
  }
}

/** Shows the current file of group `p` (or its empty state). */
function show(p) {
  const b = p.current && state.buffers.get(p.current);
  p.text.disabled = !b;
  p.empty.hidden = !!b;
  p.text.value = b ? b.content : "";
  const v = p.views.get(p.current) || { selStart: 0, selEnd: 0, scrollTop: 0, scrollLeft: 0 };
  p.text.setSelectionRange(v.selStart, v.selEnd);
  if (p === pane) emit("open");
  renderCode(p);
  p.text.scrollTop = v.scrollTop;
  p.text.scrollLeft = v.scrollLeft;
  if (p === pane) {
    caretMoved();
    if (b && !b.exists) setMessage("New file: it is created on save.", "warn");
  } else {
    withPane(p, view.scrolled);
  }
  renderTabs(p);
}

/** Opens `path` in group `p` (default: the active one), as a new tab after the current one. */
export async function openFile(path, p = pane, { focus = true } = {}) {
  if (!(await ensureBuffer(path))) return;
  if (!p.tabs.includes(path)) p.tabs.splice(p.tabs.indexOf(p.current) + 1, 0, path);
  if (focus) activate(p);
  if (p.current !== path) {
    storeView(p);
    p.current = path;
    show(p);
  }
  if (p === pane) {
    mru.splice(0, mru.length, path, ...mru.filter((m) => m !== path));
    if (focus) text.focus();
  }
  updateChrome();
  saveSession();
}

/** Sets the content of buffer `path` (reloaded from disk), keeping the selections. */
function replaceBuffer(path, content) {
  const b = state.buffers.get(path);
  if (!b) return;
  b.content = b.saved = content;
  b.exists = true;
  for (const p of state.panes) {
    if (p.current !== path) continue;
    const { selectionStart: s, selectionEnd: e, scrollTop } = p.text;
    p.text.value = content;
    p.text.setSelectionRange(Math.min(s, content.length), Math.min(e, content.length));
    p.text.scrollTop = scrollTop;
    renderCode(p);
  }
  if (state.current === path) emit("open");
}

/** Reloads the config buffer from disk after a file operation (unless it has unsaved changes). */
export async function reloadConfig() {
  const path = state.configPath;
  if (!state.buffers.has(path) || isDirty(path)) return;
  try {
    const { content } = await invoke("editor_read", { path });
    replaceBuffer(path, content);
  } catch (e) {
    setMessage(String(e), "error");
  }
}

export async function refreshFiles() {
  const list = await invoke("editor_files");
  state.files = list.files;
  const config = state.files.find((f) => f.kind === "config");
  state.configPath = config ? config.path : "";
  state.configDir = config ? config.path.replace(/[\\/][^\\/]*$/, "") : "";
  state.app = list.app;
  state.configError = list.error;
  setApp(list.app, state.configPath);
  // Clean tabs of files the config no longer lists are closed; unsaved ones stay.
  for (const p of state.panes) for (const path of [...p.tabs]) if (!fileOf(path) && !isDirty(path)) removeTab(p, path);
  updateChrome();
  if (list.error) setMessage("Saved config is invalid: " + list.error, "error");
  return list;
}

export async function saveFile(path) {
  const b = state.buffers.get(path);
  if (!b) return true;
  const content = b.content;
  try {
    await invoke("editor_write", { path, content });
  } catch (e) {
    setMessage(String(e), "error");
    return false;
  }
  b.saved = content;
  b.exists = true;
  setMessage("Saved " + baseName(path) + ". Restart to apply.", "ok");
  if (isConfig(path)) await refreshFiles().catch((e) => setMessage(String(e), "error"));
  updateChrome();
  return true;
}

const save = () => state.current && isDirty(state.current) && saveFile(state.current);

async function saveAll() {
  const dirty = [...state.buffers.keys()].filter(isDirty);
  // Scripts and styles first: saving the config may drop them from the list.
  dirty.sort((a, b) => isConfig(a) - isConfig(b));
  for (const path of dirty) if (!(await saveFile(path))) return false;
  if (dirty.length > 1) setMessage(`Saved ${dirty.length} files. Restart to apply.`, "ok");
  return true;
}

/** With unsaved changes: Save All / Don't Save / Cancel. True to go on. */
async function confirmUnsaved(action) {
  const dirty = [...state.buffers.keys()].filter(isDirty);
  if (!dirty.length) return true;
  const { button } = await modal({
    title: `Save the changes before ${action}?`,
    body: `Unsaved: ${dirty.map(baseName).join(", ")}.`,
    buttons: [
      { id: "cancel", label: "Cancel" },
      { id: "discard", label: "Don't Save" },
      { id: "save", label: "Save All", primary: true },
    ],
  });
  if (button === "save") return saveAll();
  return button === "discard";
}

async function restart() {
  if (!(await confirmUnsaved("restarting"))) return;
  try {
    await invoke("editor_restart");
    setMessage("Restarted.", "ok");
  } catch (e) {
    setMessage(String(e), "error");
  }
}

// Close button / Alt+F4 with unsaved changes (Rust prevents the close and calls this).
window.editorCloseRequested = async () => {
  if (!$("modal").hidden) return;
  if (await confirmUnsaved("closing")) invoke("editor_window", { action: "force_close" });
};

let pinned = false;

async function togglePin() {
  try {
    pinned = await invoke("editor_pin", { pinned: !pinned });
    $("pin").classList.toggle("on", pinned);
    $("pin").setAttribute("aria-pressed", pinned);
    $("pin").title = pinned ? "Unpin (stop keeping on top)" : "Keep on Top";
  } catch (e) {
    setMessage(String(e), "error");
  }
}

/** Ctrl+Tab: next tab of the active group in most-recently-used order. */
function cycleFile(dir) {
  const list = mru.filter((m) => pane.tabs.includes(m)).concat(pane.tabs.filter((t) => !mru.includes(t)));
  if (list.length < 2) return;
  const i = list.indexOf(state.current);
  openFile(list[(i + dir + list.length) % list.length]);
}

/** Ctrl+PageDown: next tab in strip order. */
function nextTab(dir) {
  const n = pane.tabs.length;
  if (n < 2) return;
  openFile(pane.tabs[(pane.tabs.indexOf(state.current) + dir + n) % n]);
}

function focusGroup(i) {
  const p = state.panes[i];
  if (p) {
    activate(p);
    if (!text.disabled) text.focus();
  } else if (i === 1) {
    splitRight();
  }
}

// ---- Session: open tabs and layout per config ----

let restoring = true;
const sessionKey = () => "weboverlay.editor.session:" + state.configPath;

function saveSession() {
  if (restoring || !state.configPath) return;
  for (const p of state.panes) if (p === pane) storeView(p);
  store(sessionKey(), {
    panes: state.panes.map((p) => ({ tabs: p.tabs, current: p.current })),
    active: state.panes.indexOf(pane),
    ratio: groupRatio,
  });
}

async function restoreSession() {
  const saved = load(sessionKey(), null);
  const listed = (path) => !!fileOf(path);
  const groups = (saved?.panes || [])
    .slice(0, 2)
    .map((g) => ({ tabs: (g.tabs || []).filter(listed), current: g.current }))
    .filter((g) => g.tabs.length);
  groupRatio = Math.min(0.85, Math.max(0.15, saved?.ratio || 0.5));
  if (!groups.length && state.files.length) groups.push({ tabs: [state.files[0].path], current: state.files[0].path });
  const active = Math.min(Math.max(saved?.active ?? 0, 0), groups.length - 1);
  for (const [i, g] of groups.entries()) {
    const p = i === 0 ? state.panes[0] : createPane();
    for (const path of g.tabs) if (await ensureBuffer(path)) p.tabs.push(path);
    const current = p.tabs.includes(g.current) ? g.current : p.tabs[0];
    if (current) await openFile(current, p, { focus: i === active || groups.length === 1 });
  }
  if (state.panes[active]) activate(state.panes[active]);
  layoutGroups();
  restoring = false;
  saveSession();
  if (!text.disabled) text.focus();
}

// ---- Commands ----

/** Editing commands (only with the text area focused). */
const E = (id, label, keys, run) => ({ id, label, keys, run, editor: true });

register([
  { id: "save", label: "File: Save", keys: ["ctrl+s"], run: save },
  { id: "saveAll", label: "File: Save All", keys: ["ctrl+k s"], run: saveAll },
  { id: "closeTab", label: "View: Close Editor", keys: ["ctrl+w", "ctrl+f4"], run: () => state.current && closeTab(pane, state.current) },
  { id: "closeAll", label: "View: Close All Editors in Group", keys: ["ctrl+k ctrl+w"], run: () => closeMany(pane, [...pane.tabs]) },
  { id: "restart", label: "WebOverlay: Restart", run: restart },
  { id: "pin", label: "View: Toggle Always on Top", run: togglePin },
  { id: "nextFile", label: "View: Open Next Recently Used Editor", keys: ["ctrl+tab"], run: () => cycleFile(1) },
  { id: "prevFile", label: "View: Open Previous Recently Used Editor", keys: ["ctrl+shift+tab"], run: () => cycleFile(-1) },
  { id: "nextTab", label: "View: Open Next Editor", keys: ["ctrl+pagedown"], run: () => nextTab(1) },
  { id: "prevTab", label: "View: Open Previous Editor", keys: ["ctrl+pageup"], run: () => nextTab(-1) },
  { id: "split", label: "View: Split Editor Right", keys: ["ctrl+\\"], run: splitRight },
  { id: "moveToOther", label: "View: Move Editor into Other Group", run: moveToOther },
  { id: "focus1", label: "View: Focus First Editor Group", keys: ["ctrl+1"], run: () => focusGroup(0) },
  { id: "focus2", label: "View: Focus Second Editor Group", keys: ["ctrl+2"], run: () => focusGroup(1) },
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

onText("keydown", (e) => {
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
    // The dialog and the context menu handle their keys themselves.
    if (e.isComposing || !$("modal").hidden || !$("menu").hidden) return;
    emit("keydown", e);
    if (e.defaultPrevented) return;
    handleKey(e, e.target === text && !text.disabled);
  },
  true,
);

onText("input", (e) => {
  const b = state.current && state.buffers.get(state.current);
  if (!b) return;
  const wasDirty = b.content !== b.saved;
  b.content = text.value;
  // The same file in the other group follows.
  for (const p of state.panes) {
    if (p === pane || p.current !== state.current) continue;
    const { selectionStart: s, selectionEnd: en, scrollTop } = p.text;
    p.text.value = b.content;
    p.text.setSelectionRange(s, en);
    p.text.scrollTop = scrollTop;
    queueRender(p);
  }
  if (wasDirty !== isDirty(state.current)) updateChrome();
  queueRender();
  caretMoved();
  emit("change", e);
});
onText("scroll", () => {
  view.scrolled();
  emit("scroll");
});
document.addEventListener("selectionchange", () => {
  if (text && document.activeElement === text) caretMoved();
});
for (const ev of ["copy", "cut", "paste"]) onText(ev, edit.clipboard);

on("openFile", (path) => openFile(path));
on("openFileIn", (path, i) => openFile(path, state.panes[i] || pane));
$("save").addEventListener("click", save);
$("restart").addEventListener("click", restart);
$("pin").addEventListener("click", togglePin);
$("pos").addEventListener("click", () => runCommand("goToLine"));

setPane(createPane());
pane.root.classList.add("active");
pane.wrap.append($("suggest"), $("hover"), $("find"));
show(pane);
document.fonts.ready.then(view.measure);
view.measure();
invoke("editor_schema")
  .then((schema) => (state.schema = schema || []))
  .catch((e) => setMessage(String(e), "error"));
refreshFiles()
  .then(restoreSession)
  .catch((e) => {
    restoring = false;
    setMessage(String(e), "error");
  });
