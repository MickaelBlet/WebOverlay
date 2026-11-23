//! Find in page (Linux): WebKitGTK has no find bar (WebView2 has its own, on Ctrl+F). Ctrl+F in
//! the content opens the header find field (`window.weboverlay.find()`), whose `find*` header
//! actions drive the content `WebKitFindController`; match counts go back to the header.

use gtk::gdk;
use gtk::glib::Propagation;
use gtk::prelude::*;
use tauri::{Manager, Webview};
use webkit2gtk::glib;
use webkit2gtk::{FindController, FindControllerExt, FindOptions, WebViewExt};

use crate::window;

/// Matches counted at most (WebKit reports `u32::MAX` beyond it).
const MAX_MATCHES: u32 = 1000;

/// Ctrl+F / F3 / Shift+F3 in the content webview and match counts sent to the header.
pub fn setup(content: &Webview) {
    let view = content.clone();
    let _ = content.with_webview(move |webview| {
        let inner = webview.inner();
        inner.connect_key_press_event({
            let view = view.clone();
            move |webview, event| {
                let state = event.state()
                    & (gdk::ModifierType::CONTROL_MASK
                        | gdk::ModifierType::SHIFT_MASK
                        | gdk::ModifierType::MOD1_MASK);
                let key = event.keyval().to_lower();
                if key == gdk::keys::constants::f && state == gdk::ModifierType::CONTROL_MASK {
                    let view = view.clone();
                    // Outside the GTK handler: Tauri calls may need the webview's lock.
                    glib::MainContext::default().spawn_local(async move { open(&view) });
                    return Propagation::Stop;
                }
                if key == gdk::keys::constants::F3 {
                    if let Some(finder) = webview.find_controller() {
                        if finder.search_text().is_some_and(|t| !t.is_empty()) {
                            if state.contains(gdk::ModifierType::SHIFT_MASK) {
                                finder.search_previous();
                            } else {
                                finder.search_next();
                            }
                            return Propagation::Stop;
                        }
                    }
                }
                Propagation::Proceed
            }
        });
        let Some(finder) = inner.find_controller() else {
            return;
        };
        finder.connect_counted_matches({
            let view = view.clone();
            move |_, count| send_count(&view, Some(count))
        });
        finder.connect_failed_to_find_text(move |_| send_count(&view, Some(0)));
    });
}

/// Opens (or focuses) the find field of the header of `content`.
fn open(content: &Webview) {
    let handle = content.app_handle();
    if let Some(header) = handle.get_webview(&window::header_label(content.window().label())) {
        let _ = header.set_focus();
        let _ = header.eval("window.weboverlay && window.weboverlay.find()");
    }
}

fn send_count(content: &Webview, count: Option<u32>) {
    let content = content.clone();
    glib::MainContext::default().spawn_local(async move {
        let handle = content.app_handle();
        let Some(header) = handle.get_webview(&window::header_label(content.window().label()))
        else {
            return;
        };
        let count = match count {
            Some(u32::MAX) => format!("\"{MAX_MATCHES}+\""),
            Some(n) => n.to_string(),
            None => "null".into(),
        };
        let _ = header.eval(format!(
            "window.weboverlay && window.weboverlay.findCount({count})"
        ));
    });
}

/// Header `find*` actions: `find` (new search for `text`, empty clears), `find_next`,
/// `find_prev`, `find_close` (clears the highlights and gives the focus back to the page).
pub fn action(content: &Webview, action: &str, text: Option<String>) -> Result<(), String> {
    let text = text.unwrap_or_default();
    let close = action == "find_close";
    let action = action.to_owned();
    let view = content.clone();
    content
        .with_webview(move |webview| {
            let Some(finder) = webview.inner().find_controller() else {
                return;
            };
            match action.as_str() {
                "find" if text.is_empty() => {
                    finder.search_finish();
                    send_count(&view, None);
                }
                "find" => search(&finder, &text),
                "find_next" => finder.search_next(),
                "find_prev" => finder.search_previous(),
                _ => finder.search_finish(),
            }
        })
        .map_err(|e| e.to_string())?;
    if close {
        let _ = content.set_focus();
    }
    Ok(())
}

fn search(finder: &FindController, text: &str) {
    let options = (FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND).bits();
    finder.count_matches(text, options, MAX_MATCHES);
    finder.search(text, options, MAX_MATCHES);
}
