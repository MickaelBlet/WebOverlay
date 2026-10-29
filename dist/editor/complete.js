// Suggest widget (completion list + documentation) and hover card; TOML via toml.js, JS/CSS here.

import { $, text, state, currentLang, isConfig, fuzzy, markHits, escapeHtml, on } from "./core.js";
import { posToXY, xyToPos, LINE_H } from "./view.js";
import { replaceRange } from "./edit.js";
import { register } from "./keymap.js";
import * as toml from "./toml.js";

// ---- JavaScript ----

const JS_KEYWORDS =
  "async await break case catch class const continue debugger default delete do else export extends false finally for function if import in instanceof let new null of return static super switch this throw true try typeof undefined var void while with yield".split(" ");

let globals = null;
const jsGlobals = () =>
  (globals ||= Object.getOwnPropertyNames(window).filter((n) => /^[A-Za-z_$][\w$]*$/.test(n) && !n.startsWith("__")));

/** Identifiers of the buffer (3+ chars) except the one being typed at `[from, to]`. */
function bufferWords(v, from, to, re = /[A-Za-z_$][\w$]{2,}/g) {
  const words = new Set();
  for (const m of v.matchAll(re)) if (m.index !== from || m.index + m[0].length !== to) words.add(m[0]);
  return [...words];
}

/** Members of the object reached by `chain` (`document.body`) from `window`, or null. */
function members(chain) {
  let obj = window;
  try {
    for (const name of chain.split(".")) {
      if (obj == null || !(name in Object(obj))) return null;
      obj = obj[name];
    }
  } catch {
    return null;
  }
  if (obj == null || (typeof obj !== "object" && typeof obj !== "function")) return null;
  const out = new Map();
  for (let o = obj; o && o !== Object.prototype; o = Object.getPrototypeOf(o)) {
    for (const name of Object.getOwnPropertyNames(o)) {
      if (out.has(name) || !/^[A-Za-z_$][\w$]*$/.test(name) || name === "constructor") continue;
      const d = Object.getOwnPropertyDescriptor(o, name);
      const method = d && typeof d.value === "function" && !/^[A-Z]/.test(name);
      out.set(name, method ? "method" : "property");
    }
  }
  return out;
}

function completeJs(v, pos) {
  let from = pos;
  while (from > 0 && /[\w$]/.test(v[from - 1])) from--;
  const lineStart = v.lastIndexOf("\n", pos - 1) + 1;
  if (/\/\/|^\s*\*/.test(v.slice(lineStart, from))) return null;
  const chain = v.slice(Math.max(0, from - 200), from).match(/([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*)\.$/);
  if (v[from - 1] === "." && chain) {
    const m = members(chain[1]);
    if (m) {
      const items = [...m]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([label, kind]) => ({ label, kind, insert: label + (kind === "method" ? "($0)" : "") }));
      return { from, to: pos, items, trigger: true };
    }
  }
  if (v[from - 1] === ".") return { from, to: pos, items: bufferWords(v, from, pos).map((label) => ({ label, kind: "word" })) };
  if (/\d/.test(v[from])) return null;
  const seen = new Set();
  const items = [];
  const add = (label, kind) => !seen.has(label) && seen.add(label) && items.push({ label, kind });
  JS_KEYWORDS.forEach((k) => add(k, "keyword"));
  bufferWords(v, from, pos).forEach((w) => add(w, "word"));
  jsGlobals().forEach((g) => add(g, /^[A-Z]/.test(g) ? "class" : "variable"));
  return { from, to: pos, items };
}

// ---- CSS ----

let cssProps = null;
function cssProperties() {
  if (cssProps) return cssProps;
  const names = new Set();
  for (const k in document.documentElement.style) {
    if (/^[a-z]/i.test(k) && !["cssText", "cssFloat", "length", "parentRule"].includes(k)) {
      const name = k.replace(/^webkit/, "-webkit-").replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
      if (typeof document.documentElement.style[k] === "string" && CSS.supports(name, "inherit")) names.add(name);
    }
  }
  return (cssProps = [...names].sort());
}

const CSS_VALUES = (
  "auto none block inline inline-block flex inline-flex grid inline-grid contents hidden visible scroll absolute " +
  "relative fixed sticky static center start end flex-start flex-end space-between space-around space-evenly stretch " +
  "baseline row column row-reverse column-reverse wrap nowrap bold normal italic underline pointer default transparent " +
  "currentcolor solid dashed dotted left right top bottom both uppercase lowercase capitalize ellipsis clip cover " +
  "contain no-repeat repeat pre pre-wrap break-word border-box content-box black white red green blue gray"
).split(" ");
const CSS_GLOBAL = ["inherit", "initial", "unset", "revert", "!important"];

function completeCss(v, pos) {
  let from = pos;
  while (from > 0 && /[\w-]/.test(v[from - 1])) from--;
  const before = v.slice(0, from).replace(/\/\*[\s\S]*?\*\//g, "");
  if (/\/\*(?![\s\S]*\*\/)/.test(before)) return null;
  const words = () => bufferWords(v, from, pos, /[A-Za-z_-][\w-]{2,}/g).map((label) => ({ label, kind: "word" }));
  if (before.lastIndexOf("{") <= before.lastIndexOf("}")) return { from, to: pos, items: words() };
  const decl = before.slice(Math.max(before.lastIndexOf("{"), before.lastIndexOf(";")) + 1);
  const colon = decl.indexOf(":");
  if (colon < 0) {
    const rest = v.slice(pos, v.indexOf("\n", pos) < 0 ? v.length : v.indexOf("\n", pos));
    const semi = rest.trim() ? "" : ";";
    const items = cssProperties().map((p) => ({ label: p, kind: "property", insert: `${p}: $0${semi}`, retrigger: true }));
    return { from, to: pos, items };
  }
  const prop = decl.slice(0, colon).trim();
  const vars = [...new Set([...v.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]))];
  const items = [
    ...CSS_VALUES.filter((x) => CSS.supports(prop, x)).map((label) => ({ label, kind: "value" })),
    ...vars.map((name) => ({ label: `var(${name})`, kind: "variable" })),
    ...CSS_GLOBAL.map((label) => ({ label, kind: "keyword" })),
  ];
  return { from, to: pos, items, trigger: true };
}

// ---- Suggest widget ----

const PROVIDERS = {
  toml: (v, pos, explicit) => (isConfig(state.current) ? toml.complete(v, pos, explicit) : null),
  js: completeJs,
  css: completeCss,
};
const ICONS = { key: "K", value: "V", table: "T", keyword: "k", word: "w", variable: "v", class: "C", method: "m", property: "p" };
const MAX_ITEMS = 200;

let active = null; // { from, to, items (filtered and sorted), selected }

export const isOpen = () => !!active;

export function close() {
  active = null;
  $("suggest").hidden = true;
}

/** Recomputes the list at the caret. `explicit`: Ctrl+Space (no minimum prefix). */
export function trigger(explicit = false) {
  const provider = PROVIDERS[currentLang()];
  const v = text.value;
  const pos = text.selectionStart;
  if (!provider || text.selectionEnd !== pos) return close();
  const result = provider(v, pos, explicit);
  const query = result && v.slice(result.from, pos);
  if (!result || (!explicit && !query && !result.trigger && !active)) return close();
  const items = [];
  for (const item of result.items) {
    const m = fuzzy(query, item.label);
    if (m && !(item.label === query && item.kind === "word")) items.push({ ...item, match: m });
  }
  // Stable: equal scores keep the provider order.
  items.sort((a, b) => b.match.score - a.match.score);
  if (!items.length) return close();
  active = { from: result.from, to: result.to, items: items.slice(0, MAX_ITEMS), selected: 0 };
  render();
}

function render() {
  const box = $("suggest");
  const list = $("suggest-list");
  list.innerHTML = active.items
    .map(
      (item, i) =>
        `<li data-i="${i}" class="${i === active.selected ? "sel" : ""}"><span class="icon k-${item.kind}">${ICONS[item.kind] || "·"}</span>` +
        `<span class="label">${markHits(item.label, item.match.hits)}</span>` +
        (item.detail ? `<span class="detail">${escapeHtml(item.detail)}</span>` : "") +
        "</li>",
    )
    .join("");
  const doc = active.items[active.selected].doc;
  $("suggest-doc").innerHTML = doc ? toml.docHtml(doc) : "";
  $("suggest-doc").hidden = !doc;
  box.hidden = false;
  // Below the caret, or above when there is no room.
  const { x, y } = posToXY(active.from);
  const wrap = $("wrap").getBoundingClientRect();
  const h = box.offsetHeight;
  const below = y + LINE_H + h <= wrap.height || y < h;
  box.style.top = `${below ? y + LINE_H : y - h}px`;
  box.style.left = `${Math.max(0, Math.min(x - 22, wrap.width - box.offsetWidth))}px`;
  list.querySelector(".sel")?.scrollIntoView({ block: "nearest" });
}

function move(delta) {
  const n = active.items.length;
  active.selected = Math.abs(delta) > 1 ? Math.max(0, Math.min(n - 1, active.selected + delta)) : (active.selected + delta + n) % n;
  render();
}

function accept(i = active.selected) {
  const item = active.items[i];
  const { from } = active;
  // Replace the typed word plus the rest of the word under the caret.
  let to = Math.max(active.to, text.selectionStart);
  const insert = item.insert ?? item.label;
  const caret = insert.indexOf("$0");
  const str = insert.replace("$0", "");
  close();
  const v = text.value;
  accepting = true;
  // Don't duplicate a closing quote / paren the snippet already has.
  if (caret >= 0 && str[caret] && v[to] === str[caret] && /[)"\]]/.test(str[caret]) && str.length === caret + 1) to++;
  try {
    replaceRange(from, to, str, from + (caret >= 0 ? caret : str.length));
  } finally {
    accepting = false;
  }
  if (item.retrigger) trigger(true);
}

// Keys while the list is open (runs before the shortcuts).
on("keydown", (e) => {
  if (!active || e.target !== text) return;
  const plain = !e.ctrlKey && !e.altKey && !e.metaKey;
  const handlers = {
    ArrowDown: () => move(1),
    ArrowUp: () => move(-1),
    PageDown: () => move(9),
    PageUp: () => move(-9),
    Enter: () => accept(),
    Tab: () => accept(),
    Escape: close,
  };
  const h = plain && !e.shiftKey && handlers[e.key];
  if (h) {
    e.preventDefault();
    e.stopPropagation();
    h();
  }
});

const WORD = /[\w$-]/;
let accepting = false;
on("change", (e) => {
  if (accepting) return;
  if (e.inputType !== "insertText" || !e.data) {
    if (active) queueMicrotask(() => trigger());
    return;
  }
  // An auto-closed pair (`""`, `[]`) is inserted at once: the typed char is the first one.
  const ch = e.data.length === 2 ? e.data[0] : e.data[e.data.length - 1];
  const triggerChar = { js: ".", toml: '"[', css: ":" }[currentLang()] || "";
  // After the current key handler: it may still move the caret (auto-closed pairs).
  if (active || WORD.test(ch) || triggerChar.includes(ch)) queueMicrotask(() => trigger());
  else close();
});
on("caret", () => {
  if (active && (text.selectionStart < active.from || text.selectionStart !== text.selectionEnd)) close();
});
on("scroll", () => active && render());
on("open", close);
text.addEventListener("blur", () => setTimeout(() => document.activeElement !== text && close(), 0));
text.addEventListener("mousedown", close);
$("suggest").addEventListener("mousedown", (e) => {
  e.preventDefault();
  const li = e.target.closest("li");
  if (li) accept(Number(li.dataset.i));
});

// ---- Hover ----

let hoverTimer = 0;
let hoverAt = null;

function hideHover() {
  clearTimeout(hoverTimer);
  hoverAt = null;
  $("hover").hidden = true;
}

/** Hover card for `pos`: the config error, then the TOML key/value docs. */
function hoverInfo(pos) {
  const v = text.value;
  const d = state.diagnostic;
  const parts = [];
  let start = pos;
  let end = pos + 1;
  if (d && d.start != null && isConfig(state.current) && pos >= d.start && pos < Math.max(d.end, d.start + 1)) {
    parts.push(`<div class="d-error">${escapeHtml(d.message)}</div>`);
    [start, end] = [d.start, d.end];
  }
  if (currentLang() === "toml" && isConfig(state.current)) {
    const h = toml.hover(v, pos);
    if (h) {
      parts.push(h.html);
      [start, end] = [h.start, h.end];
    }
  }
  return parts.length ? { html: parts.join(""), start, end } : null;
}

function showHover(pos) {
  const info = pos == null ? null : hoverInfo(pos);
  if (!info) return hideHover();
  const box = $("hover");
  box.innerHTML = info.html;
  box.hidden = false;
  const { x, y } = posToXY(info.start);
  const wrap = $("wrap").getBoundingClientRect();
  const h = box.offsetHeight;
  box.style.top = `${y - h - 2 >= 0 ? y - h - 2 : y + LINE_H + 2}px`;
  box.style.left = `${Math.max(0, Math.min(x, wrap.width - box.offsetWidth - 4))}px`;
  hoverAt = info;
}

text.addEventListener("mousemove", (e) => {
  if (e.buttons) return;
  const pos = xyToPos(e.clientX, e.clientY);
  if (hoverAt && pos != null && pos >= hoverAt.start && pos < hoverAt.end) return;
  clearTimeout(hoverTimer);
  if (hoverAt) {
    hoverAt = null;
    $("hover").hidden = true;
  }
  if (pos != null) hoverTimer = setTimeout(() => showHover(pos), 350);
});
text.addEventListener("mouseleave", hideHover);
on("keydown", hideHover);
on("scroll", hideHover);
on("open", hideHover);

register([
  { id: "suggest", label: "Trigger Suggest", keys: ["ctrl+space"], editor: true, run: () => trigger(true) },
  { id: "hover", label: "Show Hover", keys: ["ctrl+k ctrl+i"], editor: true, run: () => setTimeout(() => showHover(text.selectionStart), 0) },
]);
