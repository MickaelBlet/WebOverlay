// Text geometry (monospace, no wrap) and the decoration layer drawn under the highlighted text.

import { $, text, state } from "./core.js";

export const LINE_H = 20;
export const PAD_T = 8;
export const PAD_L = 12;
const TAB = 2;

let charW = 7.8;
export function measure() {
  const probe = document.createElement("span");
  probe.textContent = "x".repeat(100);
  $("hl").append(probe);
  charW = probe.getBoundingClientRect().width / 100 || charW;
  probe.remove();
  render();
}

// Line start offsets of the last text seen (lookups are a binary search).
let indexed = null;
let starts = [0];
function lineStarts(v) {
  if (v !== indexed) {
    indexed = v;
    starts = [0];
    for (let i = v.indexOf("\n"); i >= 0; i = v.indexOf("\n", i + 1)) starts.push(i + 1);
  }
  return starts;
}

/** `{ line, col }` (0-based, col in visual columns) of offset `pos`. */
export function lineCol(pos, v = text.value) {
  const st = lineStarts(v);
  let lo = 0;
  let hi = st.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (st[mid] <= pos) lo = mid;
    else hi = mid - 1;
  }
  let col = 0;
  for (let i = st[lo]; i < pos; i++) col += v[i] === "\t" ? TAB - (col % TAB) : 1;
  return { line: lo, col };
}

export const lineCount = (v = text.value) => lineStarts(v).length;

/** Offset of the first char of 0-based `line` (clamped). */
export function lineStart(line, v = text.value) {
  const st = lineStarts(v);
  return st[Math.max(0, Math.min(line, st.length - 1))];
}

/** Offset of the char containing visual `col` in the line starting at `start` (clamped to the line end). */
function offsetAtCol(start, col, v) {
  let c = 0;
  let i = start;
  for (; i < v.length && v[i] !== "\n"; i++) {
    const w = v[i] === "\t" ? TAB - (c % TAB) : 1;
    if (col < c + w) break;
    c += w;
  }
  return i;
}

/** Caret box of offset `pos` relative to the `#wrap` element (scroll applied). */
export function posToXY(pos) {
  const { line, col } = lineCol(pos);
  return { x: PAD_L + col * charW - text.scrollLeft, y: PAD_T + line * LINE_H - text.scrollTop, h: LINE_H };
}

/** Offset of the char under the point, or null outside the text. */
export function xyToPos(clientX, clientY) {
  const r = text.getBoundingClientRect();
  const y = clientY - r.top + text.scrollTop - PAD_T;
  const x = clientX - r.left + text.scrollLeft - PAD_L;
  const v = text.value;
  const line = Math.floor(y / LINE_H);
  if (y < 0 || x < 0 || line >= lineCount(v)) return null;
  const start = lineStart(line, v);
  const pos = offsetAtCol(start, x / charW, v);
  // Past the end of the line: no char under the point.
  return pos >= v.length || v[pos] === "\n" ? null : pos;
}

/** Scrolls so that `pos` is visible (`center`: vertically centered when it was off screen). */
export function reveal(pos, center = false) {
  const { line, col } = lineCol(pos);
  const top = PAD_T + line * LINE_H;
  const h = text.clientHeight;
  if (top < text.scrollTop + PAD_T || top + LINE_H > text.scrollTop + h - PAD_T) {
    text.scrollTop = center ? top - h / 2 : top < text.scrollTop ? top - PAD_T : top + LINE_H + PAD_T - h;
  }
  const left = PAD_L + col * charW;
  const w = text.clientWidth;
  if (left < text.scrollLeft + PAD_L) text.scrollLeft = Math.max(0, left - w / 3);
  else if (left > text.scrollLeft + w - 2 * PAD_L) text.scrollLeft = left - (2 * w) / 3;
}

// ---- Decorations: named sets of ranges, drawn as boxes in `#deco` (under `#hl`). ----

const sets = new Map(); // name -> { cls, ranges: [[start, end]] }
const MAX_BOXES = 2000;

export function decorate(name, cls, ranges) {
  if (ranges.length) sets.set(name, { cls, ranges });
  else sets.delete(name);
  queue();
}

let queued = false;
function queue() {
  if (!queued) {
    queued = true;
    requestAnimationFrame(render);
  }
}

export function render() {
  queued = false;
  const v = text.value;
  const inner = $("deco-in");
  let html = "";
  let boxes = 0;
  for (const { cls, ranges } of sets.values()) {
    for (const [start, end] of ranges) {
      // One box per line of the range; an empty range is one char wide.
      let s = Math.min(start, v.length);
      const e = Math.min(Math.max(end, start), v.length);
      while (boxes < MAX_BOXES) {
        const nl = v.indexOf("\n", s);
        const lineEnd = nl < 0 || nl >= e ? e : nl;
        const a = lineCol(s, v);
        const width = lineEnd > s ? lineCol(lineEnd, v).col - a.col : end > start ? 0.5 : 1;
        html += `<div class="${cls}" style="top:${PAD_T + a.line * LINE_H}px;left:${PAD_L + a.col * charW}px;width:${width * charW}px"></div>`;
        boxes++;
        if (lineEnd >= e) break;
        s = lineEnd + 1;
      }
    }
  }
  inner.innerHTML = html;
  scrolled();
}

/** Keeps decorations, current line and gutter aligned with the textarea scroll. */
export function scrolled() {
  $("deco-in").style.transform = `translate(${-text.scrollLeft}px, ${-text.scrollTop}px)`;
  $("hl").scrollTop = text.scrollTop;
  $("hl").scrollLeft = text.scrollLeft;
  $("gutter").scrollTop = text.scrollTop;
  const line = state.current && text.selectionStart === text.selectionEnd ? lineCol(text.selectionStart).line : -1;
  const cur = $("curline");
  cur.hidden = line < 0;
  cur.style.top = `${PAD_T + line * LINE_H - text.scrollTop}px`;
}

let gutterKey = "";
/** Line numbers, the caret line and the error line marked. */
export function renderGutter() {
  const v = text.value;
  const lines = lineCount(v);
  const cur = lineCol(text.selectionStart, v).line;
  const d = state.diagnostic;
  const err = d && d.start != null ? lineCol(d.start, v).line : -1;
  const key = `${lines}:${cur}:${err}`;
  if (key === gutterKey) return;
  gutterKey = key;
  let html = "";
  for (let i = 0; i < lines; i++) {
    const cls = i === err ? "err" : i === cur ? "cur" : "";
    html += (i ? "\n" : "") + (cls ? `<span class="${cls}">${i + 1}</span>` : i + 1);
  }
  $("gutter").innerHTML = html;
  $("gutter").scrollTop = text.scrollTop;
}

// ---- Bracket matching ----

const OPEN = "([{";
const CLOSE = ")]}";

/** Offset of the bracket matching the one at `pos`, or -1. */
export function matchBracket(pos, v = text.value) {
  const ch = v[pos];
  let dir, open, close;
  if (OPEN.includes(ch)) [dir, open, close] = [1, ch, CLOSE[OPEN.indexOf(ch)]];
  else if (CLOSE.includes(ch)) [dir, open, close] = [-1, ch, OPEN[CLOSE.indexOf(ch)]];
  else return -1;
  let depth = 0;
  for (let i = pos; i >= 0 && i < v.length; i += dir) {
    if (v[i] === open) depth++;
    else if (v[i] === close && --depth === 0) return i;
  }
  return -1;
}

export function updateBrackets() {
  const v = text.value;
  const p = text.selectionStart;
  let ranges = [];
  if (text.selectionStart === text.selectionEnd) {
    for (const at of [p - 1, p]) {
      const m = at >= 0 ? matchBracket(at, v) : -1;
      if (m >= 0) {
        ranges = [[at, at + 1], [m, m + 1]];
        break;
      }
    }
  }
  decorate("brackets", "bracket", ranges);
}
