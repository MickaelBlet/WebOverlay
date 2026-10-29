// Commands and their shortcuts (VSCode defaults), also listed by the command palette.

import { setMessage } from "./core.js";

/** `{ id, label, keys: ["ctrl+shift+k", "ctrl+k ctrl+i"], run(), editor?: only in the text area }` */
const commands = [];
export const register = (list) => commands.push(...list);
export const allCommands = () => commands.filter((c) => c.label);

// Punctuation by physical key (US layout), for layouts where the char needs Shift/AltGr (Ctrl+$ = Ctrl+]).
const US = {
  Slash: "/", Backslash: "\\", BracketLeft: "[", BracketRight: "]", Comma: ",", Period: ".",
  Semicolon: ";", Quote: "'", Backquote: "`", Minus: "-", Equal: "=",
};

function combo(e, key) {
  // Shift is part of the char for punctuation (`/` is Shift+: on AZERTY).
  const shift = e.shiftKey && !(key.length === 1 && !/[a-z0-9]/.test(key));
  return (e.ctrlKey || e.metaKey ? "ctrl+" : "") + (shift ? "shift+" : "") + (e.altKey ? "alt+" : "") + key;
}

/** Candidate key names of `e`: by produced char, then by physical key for punctuation and digits. */
function combos(e) {
  const key = e.key === " " ? "space" : e.key.toLowerCase();
  const list = [combo(e, key)];
  const physical = US[e.code] || (/^Digit\d$/.test(e.code) ? e.code[5] : null);
  if (physical && physical !== key) list.push(combo(e, physical));
  return list;
}

let chord = null;

/** Runs the command bound to `e`. `inEditor`: the text area has the focus. */
export function handleKey(e, inEditor) {
  if (["Control", "Shift", "Alt", "Meta", "AltGraph"].includes(e.key)) return false;
  const names = combos(e).map((c) => (chord ? chord + " " + c : c));
  const pending = chord;
  chord = null;
  for (const name of names) {
    const cmd = commands.find((c) => c.keys?.includes(name) && (inEditor || !c.editor));
    if (cmd) {
      e.preventDefault();
      if (pending) setMessage("");
      cmd.run();
      return true;
    }
  }
  if (pending) {
    e.preventDefault();
    setMessage(`The key combination (${label(pending)}, ${label(names[0].split(" ").pop())}) is not a command.`, "warn");
    return true;
  }
  if (!e.shiftKey && commands.some((c) => c.keys?.some((k) => k.startsWith(names[0] + " ")))) {
    e.preventDefault();
    chord = names[0];
    setMessage(`(${label(chord)}) was pressed. Waiting for second key of chord...`);
    return true;
  }
  return false;
}

const KEY_LABELS = { arrowup: "↑", arrowdown: "↓", arrowleft: "←", arrowright: "→", space: "Space", escape: "Esc" };
/** `ctrl+shift+k` -> `Ctrl+Shift+K`. */
export const label = (keys) =>
  keys
    .split(" ")
    .map((k) =>
      k
        .split("+")
        .map((p) => KEY_LABELS[p] || (p.length === 1 ? p.toUpperCase() : p[0].toUpperCase() + p.slice(1)))
        .join("+"),
    )
    .join(" ");
