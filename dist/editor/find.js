// Find / replace widget (Ctrl+F / Ctrl+H).

import { $, text, on } from "./core.js";
import { decorate, reveal } from "./view.js";
import { replaceRange, wordAt } from "./edit.js";
import { register } from "./keymap.js";

const MAX_MATCHES = 10000;
const opts = { case: false, word: false, regex: false };
let matches = []; // [[start, end]]
let current = -1;

const isOpen = () => !$("find").hidden;
const input = $("find-input");
const replaceInput = $("replace-input");

/** The search as a global RegExp, or null (empty / invalid). */
function pattern() {
  const q = input.value;
  input.classList.remove("invalid");
  if (!q) return null;
  let source = opts.regex ? q : q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  if (opts.word) source = `\\b(?:${source})\\b`;
  try {
    return new RegExp(source, "gm" + (opts.case ? "" : "i"));
  } catch {
    input.classList.add("invalid");
    return null;
  }
}

/** Recomputes the matches; `from`: select the first match at or after this offset. */
function search(from = null) {
  const re = pattern();
  const v = text.value;
  matches = [];
  if (re) {
    for (let m; matches.length < MAX_MATCHES && (m = re.exec(v)); ) {
      if (m[0].length === 0) {
        re.lastIndex++;
        continue;
      }
      matches.push([m.index, m.index + m[0].length]);
    }
  }
  if (from != null) current = matches.findIndex(([s]) => s >= from);
  if (current < 0 || current >= matches.length) current = matches.length ? 0 : -1;
  show();
}

function show() {
  decorate("find", "match", isOpen() ? matches : []);
  decorate("find-cur", "match-cur", isOpen() && current >= 0 ? [matches[current]] : []);
  const n = matches.length;
  $("find-count").textContent = !input.value ? "" : n ? `${current + 1} of ${n === MAX_MATCHES ? n + "+" : n}` : "No results";
  $("find-count").classList.toggle("none", !!input.value && !n);
  for (const k of Object.keys(opts)) $("opt-" + k).classList.toggle("on", opts[k]);
}

/** Selects the current match in the text area (visible as `.match-cur` while the widget has focus). */
function selectCurrent() {
  if (current < 0) return;
  const [s, e] = matches[current];
  text.setSelectionRange(s, e);
  reveal(s, true);
  show();
}

function go(dir) {
  if (!matches.length) return;
  current = (current + dir + matches.length) % matches.length;
  selectCurrent();
}

export function open(replace) {
  const { selectionStart: s, selectionEnd: e } = text;
  const sel = text.value.slice(s, e);
  const seed = sel && !sel.includes("\n") ? sel : !isOpen() && s === e ? text.value.slice(...wordAt(s)) : "";
  if (seed) input.value = seed;
  $("find").hidden = false;
  $("find").classList.toggle("replace", replace);
  $("replace-row").hidden = !replace;
  search(s);
  (replace && input.value ? replaceInput : input).focus();
  (replace && input.value ? replaceInput : input).select();
}

export function close() {
  if (!isOpen()) return;
  $("find").hidden = true;
  show();
  text.focus();
}

function replaceOne() {
  const re = pattern();
  if (!re || current < 0) return;
  const [s, e] = matches[current];
  const found = text.value.slice(s, e);
  const single = new RegExp(re.source, re.flags.replace("g", ""));
  const by = opts.regex ? found.replace(single, replaceInput.value) : replaceInput.value;
  replaceRange(s, e, by);
  search(s + by.length);
  selectCurrent();
  replaceInput.focus();
}

function replaceAll() {
  const re = pattern();
  if (!re || !matches.length) return;
  const v = text.value;
  const by = replaceInput.value;
  const count = matches.length;
  const result = v.replace(re, opts.regex ? by : () => by);
  // One edit: a single undo step.
  replaceRange(0, v.length, result, Math.min(text.selectionStart, result.length));
  search();
  $("find-count").textContent = `Replaced ${count}`;
  replaceInput.focus();
}

function toggle(name) {
  opts[name] = !opts[name];
  search(text.selectionStart);
}

input.addEventListener("input", () => search(text.selectionStart));
input.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.ctrlKey && !e.altKey) {
    e.preventDefault();
    go(e.shiftKey ? -1 : 1);
  }
});
replaceInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.ctrlKey && !e.altKey) {
    e.preventDefault();
    replaceOne();
  }
});
for (const k of Object.keys(opts)) $("opt-" + k).addEventListener("click", () => toggle(k));
$("find-prev").addEventListener("click", () => go(-1));
$("find-next").addEventListener("click", () => go(1));
$("find-close").addEventListener("click", close);
$("find-mode").addEventListener("click", () => open($("replace-row").hidden));
$("replace-one").addEventListener("click", replaceOne);
$("replace-all").addEventListener("click", replaceAll);
// Keep the text area focus when clicking the widget buttons.
for (const b of $("find").querySelectorAll("button")) b.addEventListener("mousedown", (e) => e.preventDefault());

on("change", () => isOpen() && search(current >= 0 ? matches[current][0] : text.selectionStart));
on("open", () => isOpen() && search(text.selectionStart));
on("keydown", (e) => {
  // Escape closes the widget from the text area too (after the suggest list).
  if (e.key === "Escape" && isOpen() && !e.defaultPrevented && (e.target === text || $("find").contains(e.target))) {
    e.preventDefault();
    if (e.target !== text && current >= 0) selectCurrent();
    close();
  }
});

const whenOpen = (fn) => () => isOpen() && fn();
register([
  { id: "find", label: "Find", keys: ["ctrl+f"], run: () => open(false) },
  { id: "replace", label: "Replace", keys: ["ctrl+h"], run: () => open(true) },
  { id: "findNext", label: "Find Next", keys: ["f3"], run: () => (isOpen() ? go(1) : open(false)) },
  { id: "findPrev", label: "Find Previous", keys: ["shift+f3"], run: () => (isOpen() ? go(-1) : open(false)) },
  { id: "replaceAll", label: null, keys: ["ctrl+alt+enter"], run: whenOpen(replaceAll) },
  { id: "toggleCase", label: null, keys: ["alt+c"], run: whenOpen(() => toggle("case")) },
  { id: "toggleWord", label: null, keys: ["alt+w"], run: whenOpen(() => toggle("word")) },
  { id: "toggleRegex", label: null, keys: ["alt+r"], run: whenOpen(() => toggle("regex")) },
]);
