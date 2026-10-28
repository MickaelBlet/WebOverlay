//! Header webview backend: per-window state pushed to `dist/header.html` and the
//! `header_action` command it invokes. Only the local header webview may call it:
//! remote content has no capability, so Tauri rejects its IPC calls anyway.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, Url, Webview};

use crate::icon;
use crate::window::{self, AppWindow};

const ZOOM_MIN: f64 = 0.3;
const ZOOM_MAX: f64 = 3.0;
const ZOOM_STEP: f64 = 0.1;

/// Evaluated in the content page: best declared icon, else `/favicon.ico`.
const FAVICON_JS: &str = r#"(() => {
  try {
    const score = (l) => {
      if (/apple-touch-icon/i.test(l.rel)) return 1;
      const size = parseInt((l.sizes && l.sizes.value) || "0", 10) || 0;
      if (size >= 16 && size <= 64) return 4;
      if (/svg/i.test(l.type) || /\.svg(\?|$)/i.test(l.href)) return 3;
      return 2;
    };
    const links = Array.from(document.querySelectorAll('link[rel~="icon" i], link[rel~="apple-touch-icon" i]'))
      .filter((l) => l.href)
      .sort((a, b) => score(b) - score(a));
    return links.length ? links[0].href : new URL("/favicon.ico", location.href).href;
  } catch (_) {
    return "";
  }
})()"#;

struct WindowState {
    app: Arc<AppWindow>,
    zoom: f64,
    title: String,
    url: String,
    favicon: String,
}

/// Window label -> state.
#[derive(Default)]
pub struct States(Mutex<HashMap<String, WindowState>>);

#[derive(Serialize)]
struct HeaderState<'a> {
    title: &'a str,
    url: &'a str,
    favicon: &'a str,
    zoom: f64,
    maximized: bool,
}

pub fn register(handle: &AppHandle, label: &str, app: Arc<AppWindow>) {
    let state = WindowState {
        url: app.config.url.to_string(),
        title: app.config.title.clone().unwrap_or_default(),
        favicon: String::new(),
        zoom: 1.0,
        app,
    };
    lock(handle).insert(label.to_owned(), state);
}

pub fn unregister(handle: &AppHandle, label: &str) {
    lock(handle).remove(label);
}

fn lock(handle: &AppHandle) -> std::sync::MutexGuard<'_, HashMap<String, WindowState>> {
    handle
        .state::<States>()
        .inner()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Records the content page title/URL and refreshes the header.
pub fn set_page(handle: &AppHandle, label: &str, title: Option<String>, url: Option<&Url>) {
    if let Some(state) = lock(handle).get_mut(label) {
        if let Some(title) = title {
            state.title = title;
        }
        if let Some(url) = url {
            state.url = url.to_string();
        }
    }
    push_state(handle, label);
}

/// Reads the content page favicon and refreshes the header when it changed.
pub fn refresh_favicon(content: &Webview) {
    let handle = content.app_handle().clone();
    let label = content.window().label().to_owned();
    let _ = content.eval_with_callback(FAVICON_JS, move |json| {
        // The page controls this value: only image-safe URLs reach the (privileged) header.
        let favicon = serde_json::from_str::<String>(&json)
            .ok()
            .and_then(|url| sanitize_favicon(&url))
            .unwrap_or_default();
        let user_agent = match lock(&handle).get_mut(&label) {
            Some(state) if state.favicon != favicon => {
                state.favicon = favicon.clone();
                Some(state.app.config.user_agent.clone())
            }
            _ => None,
        };
        if let Some(user_agent) = user_agent {
            push_state(&handle, &label);
            icon::update(&handle, &label, favicon, user_agent);
        }
    });
}

/// Current favicon URL of window `label` (empty when none).
pub fn favicon(handle: &AppHandle, label: &str) -> Option<String> {
    lock(handle).get(label).map(|state| state.favicon.clone())
}

fn sanitize_favicon(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let ok = match parsed.scheme() {
        "http" | "https" => true,
        "data" => parsed.path().starts_with("image/"),
        _ => false,
    };
    ok.then(|| parsed.to_string())
}

pub fn push_state(handle: &AppHandle, label: &str) {
    let Some(header) = handle.get_webview(&window::header_label(label)) else {
        return;
    };
    let maximized = header.window().is_maximized().unwrap_or(false);
    let js = {
        let states = lock(handle);
        let Some(state) = states.get(label) else {
            return;
        };
        let header_state = HeaderState {
            title: &state.title,
            url: &state.url,
            favicon: &state.favicon,
            zoom: state.zoom,
            maximized,
        };
        let json = serde_json::to_string(&header_state).expect("state serializes");
        format!("window.weboverlay && window.weboverlay.update({json})")
    };
    let _ = header.eval(js);
}

/// Zoom level after a step in `direction` (-1, 0 = reset, +1), clamped and rounded to the step.
fn next_zoom(current: f64, direction: i8) -> f64 {
    if direction == 0 {
        return 1.0;
    }
    let steps = (current / ZOOM_STEP).round() + f64::from(direction.signum());
    (steps * ZOOM_STEP).clamp(ZOOM_MIN, ZOOM_MAX)
}

#[tauri::command]
pub fn header_action(webview: Webview, action: String) -> Result<(), String> {
    let window = webview.window();
    let label = window.label().to_owned();
    if webview.label() != window::header_label(&label) {
        return Err("not allowed".into());
    }
    let handle = webview.app_handle();
    let content = handle
        .get_webview(&window::content_label(&label))
        .ok_or("no content webview")?;
    let result = match action.as_str() {
        "back" => content.eval("history.back()"),
        "forward" => content.eval("history.forward()"),
        "reload" => content.reload(),
        "home" => {
            let home = lock(handle).get(&label).map(|s| s.app.config.url.clone());
            match home {
                Some(url) => content.navigate(url),
                None => Ok(()),
            }
        }
        "external" => {
            if let Ok(url) = content.url() {
                window::open_external(&url);
            }
            Ok(())
        }
        "zoom_in" | "zoom_out" | "zoom_reset" => {
            let direction = match action.as_str() {
                "zoom_in" => 1,
                "zoom_out" => -1,
                _ => 0,
            };
            let zoom = {
                let mut states = lock(handle);
                let state = states.get_mut(&label).ok_or("unknown window")?;
                state.zoom = next_zoom(state.zoom, direction);
                state.zoom
            };
            let result = content.set_zoom(zoom);
            push_state(handle, &label);
            result
        }
        "minimize" => window.minimize(),
        "toggle_maximize" => match window.is_maximized() {
            Ok(true) => window.unmaximize(),
            _ => window.maximize(),
        },
        "close" => window.close(),
        "start_dragging" => window.start_dragging(),
        _ => return Err(format!("unknown action {action:?}")),
    };
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favicon_only_image_safe_urls() {
        assert_eq!(
            sanitize_favicon("https://a.com/f.ico").as_deref(),
            Some("https://a.com/f.ico")
        );
        assert!(sanitize_favicon("data:image/png;base64,AAAA").is_some());
        assert!(sanitize_favicon("data:text/html,<script>").is_none());
        assert!(sanitize_favicon("javascript:alert(1)").is_none());
        assert!(sanitize_favicon("file:///etc/passwd").is_none());
        assert!(sanitize_favicon("").is_none());
    }

    #[test]
    fn zoom_steps_and_clamps() {
        assert!((next_zoom(1.0, 1) - 1.1).abs() < 1e-9);
        assert!((next_zoom(1.0, -1) - 0.9).abs() < 1e-9);
        assert_eq!(next_zoom(1.7, 0), 1.0);
        assert_eq!(next_zoom(ZOOM_MAX, 1), ZOOM_MAX);
        assert_eq!(next_zoom(ZOOM_MIN, -1), ZOOM_MIN);
        // Repeated steps don't drift.
        let mut z = 1.0;
        for _ in 0..5 {
            z = next_zoom(z, 1);
        }
        for _ in 0..5 {
            z = next_zoom(z, -1);
        }
        assert!((z - 1.0).abs() < 1e-9);
    }
}
