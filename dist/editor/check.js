// Live validation of the config being edited (`editor_check`, the same parsing as on save).

import { $, text, invoke, state, isConfig, on } from "./core.js";
import { decorate, renderGutter, reveal } from "./view.js";

let timer = 0;
let seq = 0;

function show() {
  const d = isConfig(state.current) ? state.diagnostic : null;
  const located = d && d.start != null;
  decorate("error", "squiggle", located ? [[d.start, Math.max(d.end, d.start + 1)]] : []);
  renderGutter();
  const el = $("problem");
  el.hidden = !d;
  el.textContent = d ? "✕ " + d.message : "";
  el.title = d ? d.message + (located ? "\nClick to go to the error." : "") : "";
  el.classList.toggle("link", !!located);
}

async function check() {
  const path = state.current;
  const content = text.value;
  const n = ++seq;
  let d = null;
  try {
    d = await invoke("editor_check", { content });
  } catch (e) {
    d = { message: String(e), start: null, end: null };
  }
  // A newer check or another file won.
  if (n !== seq || path !== state.current) return;
  state.diagnostic = d;
  show();
}

const schedule = () => {
  clearTimeout(timer);
  if (!isConfig(state.current)) return show();
  timer = setTimeout(check, 400);
};

on("change", schedule);
on("open", () => {
  state.diagnostic = null;
  show();
  if (isConfig(state.current)) check();
});
$("problem").addEventListener("click", () => {
  const d = state.diagnostic;
  if (!d || d.start == null) return;
  text.focus();
  text.setSelectionRange(d.start, d.start);
  reveal(d.start, true);
});
