// Renders the documentation screenshots: the real `dist/` pages in headless Chromium with a stub
// of the Tauri IPC (fixture data below), printed to one-page PDFs, light and dark.
// Run through scripts/screenshots.sh (serves the repo, converts the PDFs to SVG).
//
// usage: node generate.mjs <base url of the repo root> <out dir> <schema.json>
// env:   CHROME = Chromium / chrome-headless-shell binary
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [base, outDir, schemaPath] = process.argv.slice(2);
if (!base || !outDir || !schemaPath || !process.env.CHROME) {
  console.error("usage: CHROME=<chromium> node generate.mjs <base url> <out dir> <schema.json>");
  process.exit(2);
}
const here = new URL(".", import.meta.url).pathname;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const dataUrl = (file) =>
  "data:image/svg+xml;base64," + readFileSync(join(here, file)).toString("base64");

// ---------------------------------------------------------------- fixtures

const DIR = "/home/user/.config/weboverlay";
const CONFIG = `[app]
name = "yoursite"
url = "https://yoursite.example.com/"
title = "Your Site"
icon = "yoursite/icon.svg"
styles = [{ path = "yoursite/style.css" }]
scripts = [
  { path = "yoursite/banner.js", run_at = "document-end", matches = ["https://yoursite.example.com/*"] },
]

[app.header]
color = "#1e3a8a"

[app.vault]
fill = "button"
`;
const BANNER = readFileSync(join(here, "../../examples/demo/banner.js"), "utf8");
const STYLE = "body {\n  font-family: system-ui, sans-serif;\n}\n";
const FILES = {
  [`${DIR}/yoursite.toml`]: CONFIG,
  [`${DIR}/yoursite/banner.js`]: BANNER,
  [`${DIR}/yoursite/style.css`]: STYLE,
};
const SITE_ICON = dataUrl("icons/yoursite.svg");

const APPS = [
  ["yoursite", "Your Site", "https://yoursite.example.com/"],
  ["mail", "Mail", "https://mail.example.com/"],
  ["chat", "Team Chat", "https://chat.example.com/"],
  ["wiki", "Wiki", "https://wiki.example.com/"],
  ["calendar", "Calendar", "https://calendar.example.com/"],
  ["monitoring", "Monitoring", "https://status.example.com/"],
];
const now = Math.floor(Date.now() / 1000);
const LAUNCHER = {
  recent: APPS.slice(0, 3).map(([name, title, url], i) => ({
    config: `${DIR}/${name}.toml`,
    app: name,
    title,
    url,
    opened: now - i * 3600,
    available: true,
    icon: dataUrl(`icons/${name}.svg`),
  })),
  configs: APPS.map(([name, title, url]) => ({
    kind: "user",
    path: `${DIR}/${name}.toml`,
    exists: true,
    error: null,
    app: { name, title, url, icon: dataUrl(`icons/${name}.svg`) },
  })),
};

const ORIGIN = "https://yoursite.example.com";
const VAULT = {
  current: ORIGIN,
  home: ORIGIN,
  logins: [
    { origin: ORIGIN, username: "alice@example.com" },
    { origin: ORIGIN, username: "bob@example.com" },
    { origin: "https://sso.example.com", username: "alice" },
  ],
};

const DOWNLOADS = [
  { id: 3, name: "export.csv", url: `${ORIGIN}/export.csv`, path: "/home/user/Downloads/export.csv", received: 3_400_000, total: 8_200_000, state: "running", asking: false },
  { id: 2, name: "report-q3.pdf", url: `${ORIGIN}/report.pdf`, path: "/home/user/Downloads/report-q3.pdf", received: 1_250_000, total: 1_250_000, state: "done", asking: false },
  { id: 1, name: "attachments.zip", url: `${ORIGIN}/files.zip`, path: "/home/user/Downloads/attachments.zip", received: 0, total: null, state: "failed", asking: false },
];

const HEADER_SETUP = {
  vars: [
    ["--bg", "#1e3a8a"],
    ["--fg", "#f3f4f6"],
    ["--muted", "rgba(243, 244, 246, 0.65)"],
    ["--hover", "rgba(243, 244, 246, 0.1)"],
    ["--active", "rgba(243, 244, 246, 0.18)"],
    ["--border", "rgba(243, 244, 246, 0.15)"],
  ],
  color_scheme: "dark",
  hide: [],
};
const HEADER_STATE = {
  title: "Your Site",
  url: `${ORIGIN}/`,
  favicon: SITE_ICON,
  zoom: 1,
  maximized: false,
  downloads: true,
};

// Init script of every frame: IPC stub, header setup, injected demo script in the site.
const stub = (extra = "") => `(() => {
  const FILES = ${JSON.stringify(FILES)};
  const SCHEMA = ${readFileSync(schemaPath, "utf8")};
  const STUBS = {
    launcher_list: ${JSON.stringify(LAUNCHER)},
    vault_list: ${JSON.stringify(VAULT)},
    downloads_list: ${JSON.stringify(DOWNLOADS)},
    editor_schema: SCHEMA,
    editor_files: {
      files: [
        { path: "${DIR}/yoursite.toml", kind: "config", app: "", exists: true },
        { path: "${DIR}/yoursite/banner.js", kind: "script", app: "yoursite", exists: true },
        { path: "${DIR}/yoursite/style.css", kind: "style", app: "yoursite", exists: true },
      ],
      app: { name: "yoursite", title: "Your Site", icon: ${JSON.stringify(SITE_ICON)} },
      error: null,
    },
    editor_read: ({ path }) => ({ content: FILES[path] ?? "", exists: path in FILES }),
    // A bare key (line without "="): the error the real check reports.
    editor_check: ({ content }) => {
      const m = /^[A-Za-z_]+$/m.exec(content);
      if (!m) return null;
      const end = m.index + m[0].length;
      return { message: "key with no value, expected \`=\`", start: end, end: end + 1 };
    },
  };
  window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      const s = STUBS[cmd];
      return typeof s === "function" ? s(args || {}) : s === undefined ? null : structuredClone(s);
    },
  };
  if (location.pathname.endsWith("/dist/header.html")) {
    window.weboverlaySetup = ${JSON.stringify(HEADER_SETUP)};
  }
  const site = location.pathname.endsWith("/site/index.html");
  addEventListener("DOMContentLoaded", () => {
    // Blurred shadows become opaque boxes in the PDF -> SVG conversion.
    const style = document.createElement("style");
    style.textContent = "*, *::before, *::after { box-shadow: none !important; text-shadow: none !important; }"
      + (site ? ${JSON.stringify(STYLE)} : "");
    document.head.append(style);
    if (site) { ${BANNER} }
  });
  ${extra}
})();`;

// ---------------------------------------------------------------- scenes

const dist = (page) => `${base}/dist/${page}`;
const SCENES = [
  {
    name: "app-window",
    url: `${base}/docs/screenshots/app.html`,
    size: [760, 440],
    run: (page) => page.eval(`document.getElementById("header").contentWindow.weboverlay.update(${JSON.stringify(HEADER_STATE)})`),
  },
  {
    name: "header",
    url: dist("header.html"),
    size: [1100, 30],
    themes: ["light"], // `[app.header] color` set: same in both themes
    run: (page) => page.eval(`window.weboverlay.update(${JSON.stringify(HEADER_STATE)})`),
  },
  { name: "launcher", url: dist("launcher.html"), size: [570, 400] },
  { name: "editor", url: dist("editor.html"), size: [1000, 700], wait: 1500 },
  {
    name: "editor-completion",
    url: dist("editor.html"),
    size: [1000, 700],
    wait: 1500,
    run: async (page) => {
      await page.eval(`(() => {
        const t = document.querySelector("textarea.text");
        t.focus();
        const i = t.value.indexOf("\\n", t.value.indexOf("title ="));
        t.setSelectionRange(i, i);
        document.dispatchEvent(new Event("selectionchange"));
      })()`);
      await page.key("Enter", "Enter", 13, "\r");
      await page.key("m", "KeyM", 77, "m");
      await page.key("a", "KeyA", 65, "a");
    },
  },
  { name: "vault", url: dist("vault.html"), size: [520, 460] },
  {
    name: "vault-add",
    url: dist("vault.html"),
    size: [520, 460],
    run: async (page) => {
      await page.eval(`document.getElementById("add").click()`);
      await sleep(200);
      await page.eval(`document.getElementById("username").focus()`);
      await page.send("Input.insertText", { text: "carol@example.com" });
      await page.eval(`document.getElementById("password").focus()`);
      await page.send("Input.insertText", { text: "correct-horse-battery" });
    },
  },
  { name: "downloads", url: dist("downloads.html"), size: [480, 420] },
];

// ---------------------------------------------------------------- CDP

const profile = mkdtempSync(join(tmpdir(), "weboverlay-shots-"));
const chrome = spawn(process.env.CHROME, [
  "--headless",
  "--no-sandbox",
  "--disable-gpu",
  "--hide-scrollbars",
  "--font-render-hinting=none",
  `--user-data-dir=${profile}`,
  "--remote-debugging-port=0",
  "about:blank",
]);
const wsUrl = await new Promise((resolve, reject) => {
  let err = "";
  chrome.stderr.on("data", (d) => {
    err += d;
    const m = /DevTools listening on (ws:\S+)/.exec(err);
    if (m) resolve(m[1]);
  });
  chrome.on("exit", (code) => reject(new Error(`chrome exited (${code}): ${err}`)));
});

const ws = new WebSocket(wsUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let nextId = 0;
const pending = new Map();
const listeners = new Set();
ws.addEventListener("message", ({ data }) => {
  const msg = JSON.parse(data);
  if (msg.id !== undefined && pending.has(msg.id)) {
    const { resolve, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    if (msg.error) reject(new Error(JSON.stringify(msg.error)));
    else resolve(msg.result);
  } else for (const l of listeners) l(msg);
});
const send = (method, params = {}, sessionId) =>
  new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params, sessionId }));
  });

async function shoot(scene, theme) {
  const [width, height] = scene.size;
  const { targetId } = await send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  const page = {
    send: (method, params) => send(method, params, sessionId),
    eval: async (expression) => {
      const r = await send("Runtime.evaluate", { expression, awaitPromise: true }, sessionId);
      if (r.exceptionDetails) throw new Error(`${scene.name}: ${r.exceptionDetails.exception?.description}`);
      return r.result.value;
    },
    key: async (key, code, keyCode, text) => {
      const p = { key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode };
      await send("Input.dispatchKeyEvent", { type: "keyDown", ...p, text }, sessionId);
      await send("Input.dispatchKeyEvent", { type: "keyUp", ...p }, sessionId);
      await sleep(150);
    },
  };
  const loaded = new Promise((resolve) => {
    const l = (msg) => {
      if (msg.sessionId === sessionId && msg.method === "Page.loadEventFired") {
        listeners.delete(l);
        resolve();
      }
    };
    listeners.add(l);
  });
  await page.send("Page.enable");
  await page.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
  await page.send("Emulation.setEmulatedMedia", {
    media: "screen",
    features: [{ name: "prefers-color-scheme", value: theme }],
  });
  await page.send("Page.addScriptToEvaluateOnNewDocument", { source: stub() });
  await page.send("Page.navigate", { url: scene.url });
  await loaded;
  await sleep(scene.wait ?? 600);
  if (scene.run) await scene.run(page);
  await sleep(600);
  const { data } = await page.send("Page.printToPDF", {
    printBackground: true,
    paperWidth: width / 96,
    paperHeight: height / 96,
    marginTop: 0,
    marginBottom: 0,
    marginLeft: 0,
    marginRight: 0,
    pageRanges: "1",
  });
  const suffix = theme === "dark" ? "-dark" : "";
  writeFileSync(join(outDir, `${scene.name}${suffix}.pdf`), Buffer.from(data, "base64"));
  await send("Target.closeTarget", { targetId });
  console.log(`${scene.name}${suffix}`);
}

try {
  const only = process.env.SCENES?.split(",");
  for (const scene of SCENES.filter((s) => !only || only.includes(s.name))) {
    for (const theme of scene.themes ?? ["light", "dark"]) await shoot(scene, theme);
  }
} finally {
  ws.close();
  chrome.kill();
  await sleep(300);
  rmSync(profile, { recursive: true, force: true });
}
