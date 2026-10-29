// Shared editor state and small helpers.

export const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
export const $ = (id) => document.getElementById(id);
export const text = $("text");

export const state = {
  files: [],
  configDir: "",
  current: null,
  /** path -> { content, saved, exists, selStart, selEnd, scrollTop, scrollLeft } */
  buffers: new Map(),
  /** Config keys documentation (`editor_schema`). */
  schema: [],
  /** Error of the config being edited: { message, start, end } (offsets may be null). */
  diagnostic: null,
};

export const escapeHtml = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export const langOf = (path) => {
  const ext = (path.match(/\.([^.\\/]+)$/) || [])[1];
  return { toml: "toml", js: "js", mjs: "js", cjs: "js", css: "css" }[(ext || "").toLowerCase()] || null;
};
export const currentLang = () => state.current && langOf(state.current);
export const LANG_NAMES = { toml: "TOML", js: "JavaScript", css: "CSS" };

export const fileOf = (path) => state.files.find((f) => f.path === path);
export const isConfig = (path) => fileOf(path)?.kind === "config";
export const relative = (path) =>
  state.configDir && path.startsWith(state.configDir)
    ? path.slice(state.configDir.length).replace(/^[\\/]+/, "")
    : path;
export const baseName = (path) => path.split(/[\\/]/).pop();

export function setMessage(msg, kind = "") {
  $("message").textContent = msg;
  $("message").className = kind;
  $("message").title = msg;
}

/** Minimal event hub: `on("change", fn)`, `emit("change")`. */
const listeners = {};
export const on = (name, fn) => (listeners[name] ||= []).push(fn);
export const emit = (name, ...args) => (listeners[name] || []).forEach((fn) => fn(...args));

/**
 * Fuzzy match of `query` in `label`: null when not every query char is found in order, else
 * `{ score, hits }` (indexes of the matched chars). Prefix > word start > contiguous > scattered.
 */
export function fuzzy(query, label) {
  if (!query) return { score: 0, hits: [] };
  const q = query.toLowerCase();
  const l = label.toLowerCase();
  if (l.startsWith(q)) return { score: 1000 - label.length, hits: [...q].map((_, i) => i) };
  const hits = [];
  let score = 0;
  let from = 0;
  for (const ch of q) {
    // Prefer a word start (after a separator or a lower→upper case change) for this char.
    let at = -1;
    for (let i = from; i < l.length; i++) {
      if (l[i] !== ch) continue;
      if (at < 0) at = i;
      const prev = label[i - 1];
      if (i === 0 || /[^A-Za-z0-9]/.test(prev) || (/[a-z]/.test(prev) && /[A-Z]/.test(label[i]))) {
        at = i;
        score += 10;
        break;
      }
    }
    if (at < 0) return null;
    if (hits.length && at === hits[hits.length - 1] + 1) score += 5;
    hits.push(at);
    from = at + 1;
  }
  return { score: score - hits[0] - label.length / 10, hits };
}

/** `label` as HTML with the `hits` indexes in `<b>`. */
export function markHits(label, hits) {
  const set = new Set(hits);
  let out = "";
  for (let i = 0; i < label.length; i++) {
    const c = escapeHtml(label[i]);
    out += set.has(i) ? `<b>${c}</b>` : c;
  }
  return out;
}
