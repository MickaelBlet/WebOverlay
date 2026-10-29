// Quick open (Ctrl+P): files; `>` commands (Ctrl+Shift+P, F1); `:` go to line (Ctrl+G).

import { $, text, state, relative, baseName, fuzzy, markHits, escapeHtml, emit } from "./core.js";
import { lineCount, lineStart, reveal } from "./view.js";
import { register, allCommands, label as keyLabel } from "./keymap.js";

const input = $("palette-input");
let items = [];
let selected = 0;

const isOpen = () => !$("palette").hidden;

function open(prefix) {
  $("palette").hidden = false;
  input.value = prefix;
  input.focus();
  update();
}

function close(refocus = true) {
  if (!isOpen()) return;
  $("palette").hidden = true;
  if (refocus && !text.disabled) text.focus();
}

/** Items for the current input: `{ label, detail, hits, run }`. */
function compute(q) {
  if (q.startsWith(">")) {
    const query = q.slice(1).trim();
    return allCommands()
      .map((c) => ({ c, m: fuzzy(query, c.label) }))
      .filter((x) => x.m)
      .sort((a, b) => b.m.score - a.m.score || a.c.label.localeCompare(b.c.label))
      .map(({ c, m }) => ({ label: c.label, hits: m.hits, detail: (c.keys || []).map(keyLabel).join("  "), run: c.run, editor: c.editor }));
  }
  if (q.startsWith(":")) {
    const lines = text.disabled ? 0 : lineCount();
    const m = q.slice(1).trim().match(/^(\d+)(?::(\d+))?$/);
    if (!m) return [{ label: `Type a line number between 1 and ${lines}`, hits: [], detail: "" }];
    const line = Math.min(Number(m[1]), lines);
    const col = Number(m[2] || 1);
    return [{ label: `Go to line ${line}` + (m[2] ? `, column ${col}` : ""), hits: [], detail: "", run: () => goTo(line, col), editor: true }];
  }
  return state.files
    .map((f) => {
      const name = f.kind === "config" ? baseName(f.path) : relative(f.path);
      return { f, name, m: fuzzy(q.trim(), name) };
    })
    .filter((x) => x.m)
    .sort((a, b) => b.m.score - a.m.score)
    .map(({ f, name, m }) => ({ label: name, hits: m.hits, detail: f.kind === "config" ? "config" : `${f.app} · ${f.kind}`, run: () => emit("openFile", f.path) }));
}

function goTo(line, col) {
  const start = lineStart(line - 1);
  const nl = text.value.indexOf("\n", start);
  const pos = Math.min(start + col - 1, nl < 0 ? text.value.length : nl);
  text.setSelectionRange(pos, pos);
  reveal(pos, true);
}

function update() {
  items = compute(input.value);
  selected = 0;
  render();
}

function render() {
  $("palette-list").innerHTML = items
    .map(
      (it, i) =>
        `<li data-i="${i}" class="${i === selected ? "sel" : ""}${it.run ? "" : " info"}"><span class="label">${markHits(it.label, it.hits)}</span>` +
        `<span class="detail">${escapeHtml(it.detail)}</span></li>`,
    )
    .join("");
  $("palette-list").querySelector(".sel")?.scrollIntoView({ block: "nearest" });
}

function run(i = selected) {
  const it = items[i];
  if (!it || !it.run) return;
  close(true);
  if (it.editor && text.disabled) return;
  it.run();
}

input.addEventListener("input", update);
input.addEventListener("keydown", (e) => {
  const n = items.length;
  const keys = {
    ArrowDown: () => n && (selected = (selected + 1) % n),
    ArrowUp: () => n && (selected = (selected - 1 + n) % n),
    PageDown: () => n && (selected = Math.min(n - 1, selected + 10)),
    PageUp: () => n && (selected = Math.max(0, selected - 10)),
  };
  if (keys[e.key]) {
    e.preventDefault();
    keys[e.key]();
    render();
  } else if (e.key === "Enter") {
    e.preventDefault();
    run();
  } else if (e.key === "Escape") {
    e.preventDefault();
    close();
  }
});
input.addEventListener("blur", () => close(false));
$("palette-list").addEventListener("mousedown", (e) => {
  e.preventDefault();
  const li = e.target.closest("li");
  if (li) run(Number(li.dataset.i));
});

register([
  { id: "quickOpen", label: "Go to File…", keys: ["ctrl+p"], run: () => open("") },
  { id: "commands", label: "Show All Commands", keys: ["ctrl+shift+p", "f1"], run: () => open(">") },
  { id: "goToLine", label: "Go to Line…", keys: ["ctrl+g"], run: () => open(":") },
]);
