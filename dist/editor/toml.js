// Config TOML: where the caret is (table, key, value), completions and hover docs from the schema.

import { state, escapeHtml } from "./core.js";

const HEADER_RE = /^[ \t]*(\[\[?)[ \t]*([A-Za-z0-9_.-]*)[ \t]*\]\]?/gm;

/** Schema table of a `[header]` / `[[header]]`. */
function headerTable(double, name) {
  if (double && name === "app") return "app";
  if (!double && name === "app.header") return "app.header";
  if (double && name === "app.scripts") return "script";
  if (double && name === "app.styles") return "style";
  return "unknown";
}

/** Table of an inline table / array of tables value of `key` in `parent`. */
function nestedTable(parent, key) {
  if (parent !== "app") return "unknown";
  return { scripts: "script", styles: "style", header: "app.header" }[key] || "unknown";
}

const isKeyChar = (c) => /[A-Za-z0-9_-]/.test(c);

/** Start of the bare key / word ending at `pos`. */
export function wordStart(v, pos) {
  let s = pos;
  while (s > 0 && isKeyChar(v[s - 1])) s--;
  return s;
}

/**
 * Syntactic context at `pos`:
 * `{ table, expect: "key"|"value"|"other", key, present: Set, string?: { start, end, closed }, comment? }`.
 * `key` is the key whose value contains `pos`; `present` the keys already set in that table.
 */
export function tomlContext(v, pos) {
  let base = "root";
  let start = 0;
  const lineStart = v.lastIndexOf("\n", pos - 1) + 1;
  HEADER_RE.lastIndex = 0;
  for (let m; (m = HEADER_RE.exec(v)) && m.index < lineStart; ) {
    base = headerTable(m[1] === "[[", m[2]);
    start = m.index + m[0].length;
  }
  const frames = [{ kind: "table", table: base, keys: new Set(), lastKey: null }];
  let expect = "key";
  const end = wordStart(v, pos);
  const top = () => frames[frames.length - 1];
  for (let i = start; i < end; i++) {
    const c = v[i];
    const f = top();
    if (c === "#") {
      const nl = v.indexOf("\n", i);
      if (nl < 0 || nl >= pos) return { table: base, expect: "other", comment: true, present: new Set() };
      i = nl - 1;
    } else if (c === '"' || c === "'") {
      const triple = v.startsWith(c.repeat(3), i);
      const q = triple ? c.repeat(3) : c;
      let j = i + q.length;
      for (; j < v.length; j++) {
        if (c === '"' && v[j] === "\\") j++;
        else if (v.startsWith(q, j)) break;
        else if (!triple && v[j] === "\n") break;
      }
      const closed = j < v.length && v.startsWith(q, j);
      if (j >= pos || (!closed && j >= end)) {
        // The caret is inside this string.
        const key = f.kind === "array" ? f.key : f.lastKey;
        return { table: tableOf(frames), expect: expect === "key" ? "key" : "value", key, present: f.keys || new Set(), inArray: f.kind === "array" ? arrayValues(v, f.start, pos) : null, string: { start: i + q.length, end: j, closed } };
      }
      if (expect === "key" && f.kind === "table") {
        f.lastKey = v.slice(i + q.length, j);
        f.keys.add(f.lastKey);
        expect = "eq";
      } else if (expect === "value") {
        expect = "other";
      }
      i = j + q.length - 1;
    } else if (c === "\n") {
      if (frames.length === 1) expect = "key";
    } else if (c === "=") {
      expect = "value";
    } else if (c === "[" && (expect === "value" || f.kind === "array")) {
      frames.push({ kind: "array", key: f.kind === "array" ? f.key : f.lastKey, parent: tableOf(frames), start: i + 1 });
      expect = "value";
    } else if (c === "{") {
      const key = f.kind === "array" ? f.key : f.lastKey;
      frames.push({ kind: "table", table: nestedTable(tableOf(frames), key), keys: new Set(), lastKey: null, inline: true });
      expect = "key";
    } else if ((c === "]" || c === "}") && frames.length > 1) {
      frames.pop();
      expect = "other";
    } else if (c === ",") {
      expect = top().kind === "table" ? "key" : "value";
    } else if (isKeyChar(c)) {
      let j = i;
      while (j < end && /[A-Za-z0-9_.+:-]/.test(v[j])) j++;
      if (expect === "key" && f.kind === "table") {
        f.lastKey = v.slice(i, j);
        f.keys.add(f.lastKey);
        expect = "eq";
      } else if (expect === "value") {
        expect = "other";
      }
      i = j - 1;
    }
  }
  const f = top();
  // Keys written after the caret in the section count as present too.
  if (frames.length === 1) {
    for (const k of sectionKeys(v, pos)) f.keys.add(k);
  }
  return {
    table: tableOf(frames),
    expect,
    key: f.kind === "array" ? f.key : f.lastKey,
    present: f.keys || new Set(),
    inArray: f.kind === "array" ? arrayValues(v, f.start, pos) : null,
  };
}

/** Effective schema table at the top of the frame stack. */
function tableOf(frames) {
  for (let i = frames.length - 1; i >= 0; i--) if (frames[i].kind === "table") return frames[i].table;
  return "unknown";
}

/** String values already listed in the array starting at `start` (before `pos`). */
function arrayValues(v, start, pos) {
  return new Set([...v.slice(start, pos).matchAll(/"([^"\n]*)"/g)].map((m) => m[1]));
}

/** Keys set at the start of a line in the section after `pos` (up to the next header). */
function sectionKeys(v, pos) {
  const nl = v.indexOf("\n", pos);
  if (nl < 0) return [];
  const rest = v.slice(nl + 1);
  const next = rest.search(/^[ \t]*\[/m);
  const section = next < 0 ? rest : rest.slice(0, next);
  return [...section.matchAll(/^[ \t]*([A-Za-z0-9_-]+)[ \t]*=/gm)].map((m) => m[1]);
}

const docOf = (table, key) => state.schema.find((k) => k.table === table && k.key === key);

// ---- Completion ----

const KIND = { key: "key", value: "value", table: "table" };

function keyItem(k) {
  return {
    label: k.key,
    kind: KIND.key,
    detail: k.type,
    doc: k,
    insert: `${k.key} = ${k.example}`,
    // Values to pick right after inserting.
    retrigger: k.values.length > 0 && k.example.includes('"$0"'),
  };
}

/** `[[app]]` / `[app.header]` snippets for a header line. */
function headerItems(v, pos) {
  const app = docOf("root", "app");
  const items = [
    { label: "[[app]]", kind: KIND.table, detail: "app window", doc: app, insert: '[[app]]\nname = "$0"\nurl = "https://"' },
  ];
  // [app.header] only inside an app that has none yet.
  const before = v.slice(0, pos);
  const appAt = before.lastIndexOf("[[app]]");
  if (appAt >= 0) {
    const after = v.slice(appAt + 7);
    const next = after.search(/^[ \t]*\[\[app\]\]/m);
    const section = next < 0 ? after : after.slice(0, next);
    if (!/^[ \t]*\[app\.header\]/m.test(section) && !/^[ \t]*header[ \t]*=/m.test(section)) {
      items.push({ label: "[app.header]", kind: KIND.table, detail: "header options", doc: docOf("app", "header"), insert: "[app.header]\n$0" });
    }
  }
  return items;
}

/** Completions at `pos`: `{ from, to, items }` or null. `explicit`: Ctrl+Space. */
export function complete(v, pos, explicit) {
  const lineStart = v.lastIndexOf("\n", pos - 1) + 1;
  const prefix = v.slice(lineStart, pos);
  // Header line: `[`, `[[ap`, …
  const header = prefix.match(/^([ \t]*)\[\[?[A-Za-z0-9_.-]*$/);
  if (header) {
    const from = lineStart + header[1].length;
    let to = pos;
    while (v[to] === "]") to++;
    return { from, to, items: headerItems(v, pos), trigger: true };
  }
  const ctx = tomlContext(v, pos);
  if (ctx.comment) return null;
  const from = wordStart(v, pos);
  if (ctx.string) {
    const doc = docOf(ctx.table, ctx.key);
    if (!doc || !doc.values.length) return null;
    const values = doc.values.filter((x) => !ctx.inArray || !ctx.inArray.has(x.value));
    const to = ctx.string.closed ? ctx.string.end : pos;
    return {
      from: ctx.string.start,
      to: Math.max(to, pos),
      items: values.map((x) => ({ label: x.value, kind: KIND.value, detail: doc.key, doc: { ...doc, valueDoc: x }, insert: x.value })),
      trigger: true,
    };
  }
  if (ctx.expect === "key") {
    if (ctx.table === "root") return explicit || from < pos ? { from, to: pos, items: headerItems(v, pos) } : null;
    const keys = state.schema.filter((k) => k.table === ctx.table && !ctx.present.has(k.key));
    return { from, to: pos, items: keys.map(keyItem) };
  }
  if (ctx.expect === "value") {
    const doc = docOf(ctx.table, ctx.key);
    if (!doc || !doc.values.length) return null;
    const quoted = doc.example.includes('"');
    const values = doc.values.filter((x) => !ctx.inArray || !ctx.inArray.has(x.value));
    return {
      from,
      to: pos,
      items: values.map((x) => ({
        label: x.value,
        kind: KIND.value,
        detail: doc.key,
        doc: { ...doc, valueDoc: x },
        insert: quoted ? `"${x.value}"` : x.value,
      })),
    };
  }
  return null;
}

// ---- Hover ----

const md = (s) => escapeHtml(s).replace(/`([^`]+)`/g, "<code>$1</code>");

/** Documentation card of a schema key (and one of its values). */
export function docHtml(doc) {
  if (!doc) return "";
  if (doc.valueDoc) {
    return (
      `<div class="d-title"><code>"${escapeHtml(doc.valueDoc.value)}"</code> <span class="d-type">${escapeHtml(doc.key)}</span></div>` +
      (doc.valueDoc.doc ? `<div class="d-doc">${md(doc.valueDoc.doc)}</div>` : "")
    );
  }
  let html = `<div class="d-title"><code>${escapeHtml(doc.key)}</code> <span class="d-type">${escapeHtml(doc.type)}${doc.required ? ", required" : ""}</span></div>`;
  html += `<div class="d-doc">${md(doc.doc)}</div>`;
  if (doc.default) html += `<div class="d-meta">Default: ${md(doc.default)}</div>`;
  if (doc.values.length && doc.values[0].doc) {
    html += `<ul class="d-values">${doc.values.map((x) => `<li><code>${escapeHtml(x.value)}</code> ${md(x.doc)}</li>`).join("")}</ul>`;
  }
  return html;
}

/** Hover card for `pos`: `{ html, start, end }` or null. */
export function hover(v, pos) {
  const lineStart = v.lastIndexOf("\n", pos - 1) + 1;
  const nl = v.indexOf("\n", pos);
  const line = v.slice(lineStart, nl < 0 ? v.length : nl);
  HEADER_RE.lastIndex = 0;
  const h = HEADER_RE.exec(line);
  if (h && pos - lineStart < h[0].length && pos - lineStart >= h[0].indexOf("[")) {
    const table = headerTable(h[1] === "[[", h[2]);
    const doc = table === "app" ? docOf("root", "app") : table === "app.header" ? docOf("app", "header") : null;
    return doc && { html: docHtml(doc), start: lineStart + h[0].indexOf("["), end: lineStart + h[0].length };
  }
  if (isKeyChar(v[pos])) {
    const s = wordStart(v, pos + 1);
    let e = pos;
    while (e < v.length && isKeyChar(v[e])) e++;
    if (/^[ \t]*=/.test(v.slice(e))) {
      const ctx = tomlContext(v, s);
      const doc = !ctx.string && ctx.expect === "key" ? docOf(ctx.table, v.slice(s, e)) : null;
      return doc && { html: docHtml(doc), start: s, end: e };
    }
  }
  // A known value inside a string.
  const ctx = tomlContext(v, pos);
  if (ctx.string) {
    const doc = docOf(ctx.table, ctx.key);
    const value = v.slice(ctx.string.start, ctx.string.end);
    const x = doc && doc.values.find((y) => y.value === value);
    return x && { html: docHtml({ ...doc, valueDoc: x }), start: ctx.string.start, end: ctx.string.end };
  }
  return null;
}
