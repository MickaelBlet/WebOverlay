// Title bar of the undecorated editor window: app identity, command center, window controls.

import { $, invoke, baseName } from "./core.js";
import { runCommand, keysOf } from "./keymap.js";

async function windowAction(action) {
  try {
    const maximized = await invoke("editor_window", { action });
    $("bar").classList.toggle("maximized", maximized);
    document.querySelector('[data-win="toggle_maximize"]').title = maximized ? "Restore" : "Maximize";
  } catch (e) {
    console.error(action, e);
  }
}

/** Shows the app of the saved config (`editor_files().app`) and the config file. */
export function setApp(app, configPath) {
  const icon = $("app-icon");
  icon.hidden = !app?.icon;
  if (app?.icon) icon.src = app.icon;
  $("brand-icon").style.display = app?.icon ? "none" : "";
  $("app-name").textContent = app ? app.title || app.name : "WebOverlay";
  $("config-name").textContent = configPath ? baseName(configPath) : "";
  document.querySelector(".brand").title = configPath;
}

for (const node of document.querySelectorAll("[data-win]")) {
  node.addEventListener("click", () => windowAction(node.dataset.win));
}

// Empty title bar area moves the window; double click toggles maximize.
$("bar").addEventListener("mousedown", (e) => {
  if (e.button !== 0 || e.target.closest("button")) return;
  windowAction(e.detail === 2 ? "toggle_maximize" : "start_dragging");
});
window.addEventListener("resize", () => windowAction("state"));
$("center").addEventListener("click", () => runCommand("quickOpen"));
// Commands are registered once every module has loaded.
queueMicrotask(() => {
  $("center").querySelector("kbd").textContent = keysOf("quickOpen");
});
windowAction("state");
