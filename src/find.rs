//! Find in page, VSCode style (match case, whole word, regex): the header find field (`#find` of
//! `dist/header.html`) drives `FIND_JS`, run in the content page. Neither WebKit's find controller
//! nor WebView2's find bar has whole word / regex matching, and the remote content has no IPC, so
//! `Ctrl+F` / `F3` are caught natively (GTK `key-press-event`, WebView2 `AcceleratorKeyPressed`,
//! which also hides the WebView2 find bar) and forwarded to the header.

use serde::{Deserialize, Serialize};
use tauri::{Manager, Webview};

use crate::window;

/// Search in the content page: `query` = a `Query` (JSON), `null` clears the highlights.
/// Visible text nodes of the main document (no shadow DOM, no iframes) are joined and matched
/// with one `RegExp`; matches become `Range`s highlighted with the CSS Custom Highlight API
/// (current one: selection without it). The page controls the returned value: Rust only keeps
/// numbers out of it.
const FIND_JS: &str = r#"((q) => {
  const KEY = Symbol.for("weboverlay.find");
  const st = window[KEY] || (window[KEY] = { key: null, index: -1, sheet: null });
  const hl = typeof Highlight === "function" && window.CSS && CSS.highlights;
  // Kept registered and emptied: WebKit doesn't repaint after `CSS.highlights.delete`.
  if (hl && !st.all) {
    st.all = new Highlight();
    st.current = new Highlight();
    st.current.priority = 1;
  }
  const clear = () => {
    if (hl) {
      st.all.clear();
      st.current.clear();
    }
  };
  clear();
  if (!q || !q.text) {
    st.key = null;
    st.index = -1;
    return { count: 0, index: -1, error: false, capped: false };
  }
  let re;
  try {
    let src = q.regex ? q.text : q.text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    if (q.word) src = `(?<![\\p{L}\\p{N}_])(?:${src})(?![\\p{L}\\p{N}_])`;
    re = new RegExp(src, q.case ? "gmu" : "gimu");
  } catch (_) {
    st.key = null;
    return { count: 0, index: -1, error: true, capped: false };
  }
  if (hl && !(st.sheet && document.adoptedStyleSheets.includes(st.sheet))) {
    try {
      st.sheet = new CSSStyleSheet();
      st.sheet.replaceSync(
        "::highlight(weboverlay-find){background-color:rgba(255,200,0,.45)}" +
        "::highlight(weboverlay-find-current){background-color:#ff9632;color:#000}");
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, st.sheet];
    } catch (_) {}
  }
  const SKIP = /^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE|TEXTAREA|SELECT|OPTION)$/;
  const seen = new Map();
  const visible = (el) => {
    let v = seen.get(el);
    if (v === undefined) {
      v = !SKIP.test(el.tagName) && (el.checkVisibility
        ? el.checkVisibility({ visibilityProperty: true })
        : el.getClientRects().length > 0);
      seen.set(el, v);
    }
    return v;
  };
  const nodes = [];
  let text = "";
  const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT);
  for (let n; (n = walker.nextNode()); ) {
    if (!n.data || !n.parentElement || !visible(n.parentElement)) continue;
    nodes.push({ n, start: text.length });
    text += n.data;
  }
  // Node holding text offset `off`: the last one starting at or before it (`end`: before it).
  const nodeAt = (off, end) => {
    let lo = 0, hi = nodes.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (end ? nodes[mid].start < off : nodes[mid].start <= off) lo = mid;
      else hi = mid - 1;
    }
    return nodes[lo];
  };
  const MAX = 10000;
  const ranges = [];
  for (const m of text.matchAll(re)) {
    if (!m[0].length) continue;
    if (ranges.length === MAX) break;
    const end = m.index + m[0].length;
    const a = nodeAt(m.index, false), b = nodeAt(end, true);
    const r = document.createRange();
    r.setStart(a.n, m.index - a.start);
    r.setEnd(b.n, end - b.start);
    ranges.push(r);
  }
  const key = JSON.stringify([q.text, q.case, q.word, q.regex]);
  let index = -1;
  if (ranges.length) {
    if (key === st.key && st.index >= 0) {
      index = (Math.min(st.index, ranges.length - 1) + q.step + ranges.length) % ranges.length;
    } else {
      // New search: first match from the top of the viewport.
      index = Math.max(0, ranges.findIndex((r) => r.getBoundingClientRect().bottom >= 0));
    }
  }
  st.key = key;
  st.index = index;
  if (index < 0) return { count: 0, index: -1, error: false, capped: false };
  const current = ranges[index];
  if (hl) {
    for (const r of ranges) st.all.add(r);
    st.current.add(current);
    // Registered again in case the page cleared `CSS.highlights`.
    CSS.highlights.set("weboverlay-find", st.all);
    CSS.highlights.set("weboverlay-find-current", st.current);
  } else {
    const sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(current);
  }
  if (q.scroll) {
    const out = (r) => r.top < 0 || r.bottom > innerHeight || r.left < 0 || r.right > innerWidth;
    if (out(current.getBoundingClientRect())) {
      const el = current.startContainer.parentElement;
      if (el) el.scrollIntoView({ block: "center", inline: "nearest" });
      // A large element: center the match itself.
      const r = current.getBoundingClientRect();
      if (out(r)) scrollBy(r.left < 0 || r.right > innerWidth ? r.left - innerWidth / 2 : 0, r.top - innerHeight / 2);
    }
  }
  return { count: ranges.length, index, error: false, capped: ranges.length === MAX };
})"#;

/// Search sent by the header (`header_action` `find` argument).
#[derive(Deserialize, Serialize)]
pub struct Query {
    text: String,
    case: bool,
    word: bool,
    regex: bool,
    /// 0 = search again / new search, 1 = next match, -1 = previous one.
    step: i8,
    /// Scrolls to the current match (not when refreshing after a page load).
    scroll: bool,
    /// Echoed back: the header ignores results of older searches.
    seq: u32,
}

#[derive(Deserialize, Serialize)]
struct Found {
    count: u32,
    index: i64,
    error: bool,
    capped: bool,
}

#[derive(Serialize)]
struct Reply {
    seq: u32,
    #[serde(flatten)]
    found: Found,
}

/// Header `find*` actions: `find` (runs `query`), `find_close` (clears the highlights and gives
/// the focus back to the page).
pub fn action(content: &Webview, action: &str, query: Option<Query>) -> Result<(), String> {
    match action {
        "find" => {
            let query = query.ok_or("missing query")?;
            let seq = query.seq;
            let json = serde_json::to_string(&query).expect("query serializes");
            let view = content.clone();
            content
                .eval_with_callback(format!("{FIND_JS}({json})"), move |result| {
                    let found = serde_json::from_str::<Found>(&result).unwrap_or(Found {
                        count: 0,
                        index: -1,
                        error: false,
                        capped: false,
                    });
                    reply(&view, &Reply { seq, found });
                })
                .map_err(|e| e.to_string())
        }
        _ => {
            let _ = content.eval(format!("{FIND_JS}(null)"));
            content.set_focus().map_err(|e| e.to_string())
        }
    }
}

fn header(content: &Webview) -> Option<Webview> {
    content
        .app_handle()
        .get_webview(&window::header_label(content.window().label()))
}

fn reply(content: &Webview, reply: &Reply) {
    if let Some(header) = header(content) {
        let json = serde_json::to_string(reply).expect("reply serializes");
        let _ = header.eval(format!(
            "window.weboverlay && window.weboverlay.findResult({json})"
        ));
    }
}

/// Key caught in the content page.
enum Key {
    /// `Ctrl+F`: opens (focuses) the find field.
    Open,
    /// `F3` / `Shift+F3`: next / previous match.
    Step(i8),
}

fn key_pressed(content: &Webview, key: Key) {
    let Some(header) = header(content) else {
        return;
    };
    let js = match key {
        Key::Open => {
            let _ = header.set_focus();
            "window.weboverlay && window.weboverlay.find()".to_owned()
        }
        Key::Step(step) => format!("window.weboverlay && window.weboverlay.findStep({step})"),
    };
    let _ = header.eval(js);
}

/// Content page loaded: the header searches it again when its find field is open.
pub fn page_loaded(content: &Webview) {
    if let Some(header) = header(content) {
        let _ = header.eval("window.weboverlay && window.weboverlay.findRefresh()");
    }
}

/// Catches `Ctrl+F` / `F3` / `Shift+F3` in the content webview.
pub fn setup(content: &Webview) {
    let view = content.clone();
    #[cfg(target_os = "linux")]
    let _ = content.with_webview(move |webview| linux::setup(&webview.inner(), view));
    #[cfg(windows)]
    let _ = content.with_webview(move |webview| webview2::setup(&webview.controller(), view));
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = view;
}

#[cfg(target_os = "linux")]
mod linux {
    use gtk::gdk;
    use gtk::glib::Propagation;
    use gtk::prelude::*;
    use tauri::Webview;
    use webkit2gtk::{glib, WebView};

    use super::Key;

    pub fn setup(view: &WebView, content: Webview) {
        view.connect_key_press_event(move |_, event| {
            let mods = event.state()
                & (gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::SHIFT_MASK
                    | gdk::ModifierType::MOD1_MASK);
            let keyval = event.keyval().to_lower();
            let key =
                if keyval == gdk::keys::constants::f && mods == gdk::ModifierType::CONTROL_MASK {
                    Key::Open
                } else if keyval == gdk::keys::constants::F3
                    && !mods.contains(gdk::ModifierType::CONTROL_MASK)
                {
                    Key::Step(if mods.contains(gdk::ModifierType::SHIFT_MASK) {
                        -1
                    } else {
                        1
                    })
                } else {
                    return Propagation::Proceed;
                };
            let content = content.clone();
            // Outside the GTK handler: Tauri calls may need the webview's lock.
            glib::MainContext::default()
                .spawn_local(async move { super::key_pressed(&content, key) });
            Propagation::Stop
        });
    }
}

#[cfg(windows)]
mod webview2 {
    use tauri::Webview;
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller, COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_F, VK_F3, VK_MENU, VK_SHIFT,
    };

    use super::Key;

    pub fn setup(controller: &ICoreWebView2Controller, content: Webview) {
        let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let (mut kind, mut vk) = (COREWEBVIEW2_KEY_EVENT_KIND::default(), 0u32);
            unsafe {
                args.KeyEventKind(&mut kind)?;
                args.VirtualKey(&mut vk)?;
            }
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN {
                return Ok(());
            }
            let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
            let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
            let key = if vk == u32::from(VK_F.0) && ctrl && !shift && !alt {
                Key::Open
            } else if vk == u32::from(VK_F3.0) && !ctrl && !alt {
                Key::Step(if shift { -1 } else { 1 })
            } else {
                return Ok(());
            };
            // Also hides the WebView2 find bar.
            unsafe { args.SetHandled(true)? };
            let content = content.clone();
            // Not inside the event: Tauri calls may run inline on the main thread.
            let _ = content
                .clone()
                .run_on_main_thread(move || super::key_pressed(&content, key));
            Ok(())
        }));
        if let Err(e) = unsafe { controller.add_AcceleratorKeyPressed(&handler, &mut 0) } {
            eprintln!("weboverlay: cannot catch the find shortcuts: {e}");
        }
    }
}
