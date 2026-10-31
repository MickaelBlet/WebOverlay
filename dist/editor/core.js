// Shared editor state and small helpers.

export const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
export const $ = (id) => document.getElementById(id);

/**
 * The active editor group: `{ root, tabsEl, text, hl, hlCode, gutter, curline, decoIn, wrap, empty,
 * tabs: [path], current, views: Map(path -> { selStart, selEnd, scrollTop, scrollLeft }) }`.
 * `text` is its text area. Both are live bindings: they follow `setPane`.
 */
export let pane = null;
export let text = null;

export function setPane(p) {
  pane = p;
  text = p ? p.text : null;
}

/** Runs `fn` with `p` as the active group (rendering of an inactive group). */
export function withPane(p, fn) {
  const prev = pane;
  setPane(p);
  try {
    return fn();
  } finally {
    setPane(prev);
  }
}

const textListeners = [];
const listenText = (p, type, fn) =>
  p.text.addEventListener(type, (e) => {
    if (p === pane) fn(e);
  });
/** Listener on the text area of every group (current and future), called for the active one only. */
export function onText(type, fn) {
  textListeners.push([type, fn]);
  for (const p of state.panes) listenText(p, type, fn);
}
/** Adds the `onText` listeners to a new group. */
export const attachText = (p) => textListeners.forEach(([type, fn]) => listenText(p, type, fn));

export const state = {
  files: [],
  configDir: "",
  configPath: "",
  /** Editor groups (one or two). */
  panes: [],
  /** path -> { content, saved, exists } */
  buffers: new Map(),
  /** Config keys documentation (`editor_schema`). */
  schema: [],
  /** Error of the config being edited: { message, start, end } (offsets may be null). */
  diagnostic: null,
};
/** File of the active group. */
Object.defineProperty(state, "current", {
  get: () => pane?.current ?? null,
  set: (path) => {
    if (pane) pane.current = path;
  },
  enumerable: true,
});

export const isDirty = (path) => {
  const b = state.buffers.get(path);
  return !!b && b.content !== b.saved;
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
/** Directory part of `relative(path)` (empty at the config directory). */
export const relDir = (path) => relative(path).replace(/[^\\/]*$/, "").replace(/[\\/]$/, "");

/** Element helper: `el("div", { className: "x", onclick }, child, "text")`. */
export function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null) continue;
    if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else if (k === "dataset") Object.assign(node.dataset, v);
    else if (k in node) node[k] = v;
    else node.setAttribute(k, v);
  }
  node.append(...children.filter((c) => c != null));
  return node;
}

/** `localStorage` JSON value (absent or blocked storage: `fallback`). */
export function load(key, fallback) {
  try {
    const v = localStorage.getItem(key);
    return v == null ? fallback : JSON.parse(v);
  } catch {
    return fallback;
  }
}
export function store(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Storage blocked: the layout is just not remembered.
  }
}

export async function copyText(str) {
  try {
    await navigator.clipboard.writeText(str);
  } catch {
    const t = el("textarea", { value: str });
    document.body.append(t);
    t.select();
    document.execCommand("copy");
    t.remove();
  }
}

// ---- Context menu ----

let menuReturn = null;
/**
 * Shows a context menu at (x, y): `entries` = `{ label, run, keys?, danger?, disabled? }` or
 * `"-"` (separator).
 */
export function showMenu(x, y, entries) {
  const menu = $("menu");
  menuReturn = document.activeElement;
  menu.replaceChildren(
    ...entries.map((e) =>
      e === "-"
        ? el("hr")
        : el(
            "div",
            {
              className: "item" + (e.danger ? " danger" : "") + (e.disabled ? " disabled" : ""),
              role: "menuitem",
              onmousedown: (ev) => ev.preventDefault(),
              onclick: () => {
                hideMenu();
                e.run();
              },
            },
            el("span", { textContent: e.label }),
            e.keys ? el("span", { className: "k", textContent: e.keys }) : null,
          ),
    ),
  );
  menu.hidden = false;
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(4, Math.min(x, innerWidth - r.width - 4))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, innerHeight - r.height - 4))}px`;
}
export function hideMenu() {
  if ($("menu").hidden) return;
  $("menu").hidden = true;
  menuReturn?.focus?.();
}
document.addEventListener("mousedown", (e) => {
  if (!$("menu").contains(e.target)) hideMenu();
});
window.addEventListener("blur", hideMenu);
window.addEventListener("resize", hideMenu);
document.addEventListener(
  "keydown",
  (e) => {
    const menu = $("menu");
    if (menu.hidden) return;
    const items = [...menu.querySelectorAll(".item:not(.disabled)")];
    let i = items.findIndex((it) => it.classList.contains("sel"));
    const go = (d) => {
      items[i]?.classList.remove("sel");
      i = (i + d + items.length) % items.length;
      items[i]?.classList.add("sel");
    };
    const keys = { ArrowDown: () => go(1), ArrowUp: () => go(i < 0 ? 0 : -1), Escape: hideMenu, Enter: () => items[i]?.click() };
    if (keys[e.key]) {
      e.preventDefault();
      e.stopPropagation();
      keys[e.key]();
    } else if (!["Control", "Shift", "Alt", "Meta"].includes(e.key)) {
      hideMenu();
    }
  },
  true,
);

// ---- Modal dialog ----

/**
 * Shows a modal dialog; resolves with `{ button, values }` (`button` = the id of the clicked
 * button, `null` when dismissed). `buttons`: `[{ id, label, primary?, danger? }]` (the first
 * primary one is the default, Enter). `fields`: `[{ id, label, value, options?: [[value, label]],
 * hint?, placeholder?, selection?: [start, end] }]` (the first field gets the focus). `validate(values, form)` returns an error message or "".
 */
export function modal({ title, body = "", fields = [], buttons, validate, onInput }) {
  const box = $("modal");
  const form = box.querySelector("form");
  const prevFocus = document.activeElement;
  const inputs = {};
  const error = el("div", { className: "error" });
  const grid = fields.length
    ? el(
        "div",
        { className: "fields" },
        ...fields.flatMap((f) => {
          const input = f.options
            ? el("select", { id: "f-" + f.id }, ...f.options.map(([v, l]) => el("option", { value: v, textContent: l })))
            : el("input", { id: "f-" + f.id, spellcheck: false, autocomplete: "off", placeholder: f.placeholder || "" });
          input.value = f.value ?? "";
          inputs[f.id] = input;
          return [
            el("label", { htmlFor: "f-" + f.id, textContent: f.label }),
            input,
            f.hint ? el("div", { className: "hint", textContent: f.hint }) : null,
          ];
        }),
      )
    : null;
  const values = () => Object.fromEntries(Object.entries(inputs).map(([k, i]) => [k, i.value]));
  return new Promise((resolve) => {
    const done = (button) => {
      if (button) {
        const msg = validate && buttons.find((b) => b.id === button)?.primary ? validate(values()) : "";
        if (msg) {
          error.textContent = msg;
          return;
        }
      }
      box.hidden = true;
      document.removeEventListener("keydown", onKey, true);
      prevFocus?.focus?.();
      resolve({ button, values: values() });
    };
    const onKey = (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        done(null);
      } else if (e.key === "Enter" && e.target.tagName !== "BUTTON") {
        e.preventDefault();
        e.stopPropagation();
        const def = buttons.find((b) => b.primary);
        if (def) done(def.id);
      } else {
        // Shortcuts stay off while the dialog is open.
        e.stopPropagation();
      }
    };
    form.replaceChildren(
      ...[el("h2", { textContent: title }), body ? el("p", { textContent: body }) : null, grid, validate ? error : null].filter(Boolean),
      el(
        "div",
        { className: "buttons" },
        ...buttons.map((b) =>
          el("button", {
            type: "button",
            className: b.primary ? "primary" : b.danger ? "danger" : "",
            textContent: b.label,
            onclick: () => done(b.id),
          }),
        ),
      ),
    );
    if (onInput) {
      for (const input of Object.values(inputs)) {
        input.addEventListener("input", () => {
          error.textContent = "";
          onInput(values(), inputs);
        });
      }
    }
    box.onmousedown = (e) => e.target === box && done(null);
    box.hidden = false;
    document.addEventListener("keydown", onKey, true);
    onInput?.(values(), inputs);
    const first = fields[0] && inputs[fields[0].id];
    if (first) {
      first.focus();
      const [s, e] = fields[0].selection || [0, first.value.length];
      first.setSelectionRange?.(s, e);
    } else {
      (form.querySelector("button.primary") || form.querySelector("button"))?.focus();
    }
  });
}

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
