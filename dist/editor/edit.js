// Editing commands. Every change goes through `replaceRange` (execCommand keeps the native undo stack).

import { text, currentLang } from "./core.js";
import { reveal } from "./view.js";

export const INDENT = "  ";

export function replaceRange(start, end, str, selStart = start + str.length, selEnd = selStart) {
  text.focus();
  text.setSelectionRange(start, end);
  if (str) document.execCommand("insertText", false, str);
  else if (start !== end) document.execCommand("delete");
  text.setSelectionRange(Math.min(selStart, selEnd), Math.max(selStart, selEnd), selEnd < selStart ? "backward" : "forward");
}

export const select = (start, end = start) => {
  text.setSelectionRange(start, end);
  reveal(end);
};

/** Full lines covered by the selection: `{ ls, le }` (`le` = end of the last line, before its `\n`). */
export function lineRange(start = text.selectionStart, end = text.selectionEnd, v = text.value) {
  const ls = v.lastIndexOf("\n", start - 1) + 1;
  // A selection ending at the start of a line doesn't include that line.
  const last = end > start && v[end - 1] === "\n" ? end - 1 : end;
  const nl = v.indexOf("\n", last);
  return { ls, le: nl < 0 ? v.length : nl };
}

const indentOf = (line) => line.match(/^[ \t]*/)[0];

/** The selection's moving end (the caret). */
export const caret = () => (text.selectionDirection === "backward" ? text.selectionStart : text.selectionEnd);
const anchor = () => (text.selectionDirection === "backward" ? text.selectionEnd : text.selectionStart);

/** Word (`[\w$-]` in CSS, `[\w$]` elsewhere) around `pos`: `[start, end]`. */
export function wordAt(pos, v = text.value) {
  const re = currentLang() === "css" ? /[\w$-]/ : /[\w$]/;
  let s = pos;
  let e = pos;
  while (s > 0 && re.test(v[s - 1])) s--;
  while (e < v.length && re.test(v[e])) e++;
  return [s, e];
}

// ---- Indentation ----

/** Tab: spaces up to the next tab stop without selection, else indent the selected lines. */
export function tab(outdent) {
  if (!outdent && text.selectionStart === text.selectionEnd) {
    const v = text.value;
    const p = text.selectionStart;
    const col = p - (v.lastIndexOf("\n", p - 1) + 1);
    replaceRange(p, p, " ".repeat(INDENT.length - (col % INDENT.length)));
    return;
  }
  indentLines(outdent);
}

export function indentLines(outdent) {
  const v = text.value;
  const { selectionStart: start, selectionEnd: end } = text;
  const { ls, le } = lineRange(start, end, v);
  const lines = v.slice(ls, le).split("\n");
  const changed = lines.map((l) => (outdent ? l.replace(/^(?: {1,2}|\t)/, "") : l.length ? INDENT + l : l));
  const block = changed.join("\n");
  if (block === v.slice(ls, le)) return;
  const first = changed[0].length - lines[0].length;
  const total = block.length - (le - ls);
  // A selection starting at the line start keeps covering the whole line.
  const from = start === ls && start !== end ? ls : Math.max(ls, start + first);
  replaceRange(ls, le, block, from, start === end ? from : end + total);
}

/** Enter: keeps the indentation, indents after an opening bracket, splits `{|}` into a block. */
export function newline() {
  const v = text.value;
  const { selectionStart: start, selectionEnd: end } = text;
  const lineStart = v.lastIndexOf("\n", start - 1) + 1;
  const ws = indentOf(v.slice(lineStart)).slice(0, start - lineStart);
  const before = v.slice(lineStart, start).trimEnd();
  const open = before[before.length - 1];
  const next = v[end];
  if (open && "([{".includes(open)) {
    const inner = "\n" + ws + INDENT;
    const split = next === { "(": ")", "[": "]", "{": "}" }[open];
    replaceRange(start, end, inner + (split ? "\n" + ws : ""), start + inner.length);
  } else {
    replaceRange(start, end, "\n" + ws);
  }
}

/** Home: first non-blank char, or the line start when already there. */
export function home(extend) {
  const v = text.value;
  const p = caret();
  const ls = v.lastIndexOf("\n", p - 1) + 1;
  const first = ls + indentOf(v.slice(ls)).length;
  const target = p === first ? ls : first;
  const a = extend ? anchor() : target;
  text.setSelectionRange(Math.min(a, target), Math.max(a, target), target < a ? "backward" : "forward");
  reveal(target);
}

// ---- Line operations ----

export function moveLines(dir) {
  const v = text.value;
  const { selectionStart: s, selectionEnd: e } = text;
  const { ls, le } = lineRange(s, e, v);
  const block = v.slice(ls, le);
  if (dir < 0) {
    if (ls === 0) return;
    const prev = v.lastIndexOf("\n", ls - 2) + 1;
    const shift = ls - prev;
    replaceRange(prev, le, block + "\n" + v.slice(prev, ls - 1), s - shift, e - shift);
  } else {
    if (le >= v.length) return;
    const nl = v.indexOf("\n", le + 1);
    const nextEnd = nl < 0 ? v.length : nl;
    const next = v.slice(le + 1, nextEnd);
    replaceRange(ls, nextEnd, next + "\n" + block, s + next.length + 1, e + next.length + 1);
  }
  reveal(text.selectionStart);
}

export function copyLines(dir) {
  const v = text.value;
  const { selectionStart: s, selectionEnd: e } = text;
  const { ls, le } = lineRange(s, e, v);
  const block = v.slice(ls, le);
  if (dir < 0) replaceRange(ls, ls, block + "\n", s, e);
  else replaceRange(le, le, "\n" + block, s + block.length + 1, e + block.length + 1);
  reveal(text.selectionStart);
}

export function deleteLines() {
  const v = text.value;
  const { ls, le } = lineRange();
  const col = text.selectionStart - ls;
  if (le < v.length) {
    replaceRange(ls, le + 1, "");
  } else {
    replaceRange(Math.max(0, ls - 1), le, "");
  }
  // Same column on the line now under the caret.
  const w = text.value;
  const start = w.lastIndexOf("\n", Math.min(ls, w.length) - 1) + 1;
  const nl = w.indexOf("\n", start);
  select(Math.min(start + col, nl < 0 ? w.length : nl));
}

export function insertLine(above) {
  const v = text.value;
  const { ls, le } = lineRange(text.selectionStart, text.selectionStart, v);
  const ws = indentOf(v.slice(ls, le));
  if (above) replaceRange(ls, ls, ws + "\n", ls + ws.length);
  else replaceRange(le, le, "\n" + ws);
}

/** Ctrl+L: selects the line, then extends to the next ones. */
export function selectLine() {
  const v = text.value;
  const { selectionStart: s, selectionEnd: e } = text;
  const full = e > s && (s === 0 || v[s - 1] === "\n") && (e === v.length || v[e - 1] === "\n");
  const { ls } = lineRange(s, s, v);
  const from = full ? e : s;
  const nl = v.indexOf("\n", from);
  select(full ? s : ls, nl < 0 ? v.length : nl + 1);
}

/** Ctrl+D: selects the word under the caret, then its next occurrence. */
export function selectNextOccurrence() {
  const v = text.value;
  const { selectionStart: s, selectionEnd: e } = text;
  if (s === e) {
    const [ws, we] = wordAt(s, v);
    if (ws < we) select(ws, we);
    return;
  }
  const needle = v.slice(s, e);
  let at = v.indexOf(needle, e);
  if (at < 0) at = v.indexOf(needle);
  if (at >= 0) select(at, at + needle.length);
}

// ---- Comments ----

const LINE_COMMENT = { toml: "#", js: "//" };

export function toggleLineComment() {
  const token = LINE_COMMENT[currentLang()];
  if (!token) {
    if (currentLang() === "css") toggleBlockComment(true);
    return;
  }
  const v = text.value;
  const { selectionStart: s, selectionEnd: e } = text;
  const { ls, le } = lineRange(s, e, v);
  const lines = v.slice(ls, le).split("\n");
  const used = lines.filter((l) => l.trim());
  if (!used.length) return;
  const commented = used.every((l) => l.trimStart().startsWith(token));
  const minIndent = Math.min(...used.map((l) => indentOf(l).length));
  const changed = lines.map((l) => {
    if (!l.trim()) return l;
    if (commented) {
      const ws = indentOf(l);
      return ws + l.slice(ws.length + token.length).replace(/^ /, "");
    }
    return l.slice(0, minIndent) + token + " " + l.slice(minIndent);
  });
  const block = changed.join("\n");
  const first = changed[0].length - lines[0].length;
  const total = block.length - (le - ls);
  const from = s === ls && s !== e ? ls : Math.max(ls, s + first);
  replaceRange(ls, le, block, from, s === e ? from : e + total);
}

/** `/* … *\/` around the selection (`lines`: around the selected lines), or removes it. */
export function toggleBlockComment(lines = false) {
  if (!["js", "css"].includes(currentLang())) return;
  const v = text.value;
  let { selectionStart: s, selectionEnd: e } = text;
  if (lines) {
    const r = lineRange(s, e, v);
    s = r.ls + indentOf(v.slice(r.ls, r.le)).length;
    e = r.le;
  }
  const sel = v.slice(s, e);
  const m = sel.match(/^\/\*\s?([\s\S]*?)\s?\*\/$/);
  if (m) {
    replaceRange(s, e, m[1], s, s + m[1].length);
  } else if (s === e) {
    replaceRange(s, s, "/*  */", s + 3);
  } else {
    replaceRange(s, e, "/* " + sel + " */", s, e + 6);
  }
}

// ---- Bracket and quote pairs ----

const PAIRS = {
  toml: { "[": "]", "{": "}", '"': '"', "'": "'" },
  js: { "(": ")", "[": "]", "{": "}", '"': '"', "'": "'", "`": "`" },
  css: { "(": ")", "[": "]", "{": "}", '"': '"', "'": "'" },
};

/** Handles a typed char: wraps the selection, auto-closes or overtypes. True when handled. */
export function typeChar(ch) {
  const pairs = PAIRS[currentLang()];
  if (!pairs) return false;
  const v = text.value;
  const { selectionStart: s, selectionEnd: e } = text;
  const closers = Object.values(pairs);
  const quote = pairs[ch] === ch;
  if (s !== e) {
    if (!pairs[ch]) return false;
    replaceRange(s, e, ch + v.slice(s, e) + pairs[ch], s + 1, e + 1);
    return true;
  }
  if (closers.includes(ch) && v[s] === ch && (!quote || v[s - 1] !== "\\")) {
    text.setSelectionRange(s + 1, s + 1);
    return true;
  }
  if (!pairs[ch]) return false;
  const next = v[s];
  if (next !== undefined && !/[\s)\]},;:]/.test(next)) return false;
  if (quote && /[\w\\]/.test(v[s - 1] || "")) return false;
  replaceRange(s, s, ch + pairs[ch], s + 1);
  return true;
}

/** Backspace between an empty pair deletes both. True when handled. */
export function backspacePair() {
  const pairs = PAIRS[currentLang()];
  const v = text.value;
  const { selectionStart: s, selectionEnd: e } = text;
  if (!pairs || s !== e || s === 0 || pairs[v[s - 1]] !== v[s]) return false;
  replaceRange(s - 1, s + 1, "");
  return true;
}

// ---- Whole-line clipboard (Ctrl+C / Ctrl+X without selection) ----

let lineClip = null;

export function clipboard(e) {
  const { selectionStart: s, selectionEnd: en } = text;
  if (e.type === "paste") {
    const data = e.clipboardData.getData("text/plain");
    // A whole line copied without selection is pasted above the caret line.
    if (s === en && data === lineClip) {
      e.preventDefault();
      const ls = text.value.lastIndexOf("\n", s - 1) + 1;
      replaceRange(ls, ls, data, s + data.length);
    }
    return;
  }
  if (s !== en) {
    lineClip = null;
    return;
  }
  const v = text.value;
  const { ls, le } = lineRange(s, s, v);
  lineClip = v.slice(ls, le) + "\n";
  e.preventDefault();
  e.clipboardData.setData("text/plain", lineClip);
  if (e.type === "cut") deleteLines();
}
