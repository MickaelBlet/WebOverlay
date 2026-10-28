//! App windows: an undecorated window holding the header webview (local, `dist/header.html`)
//! above the content webview (the remote site). Popups are plain decorated webview windows.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tauri::webview::{NewWindowFeatures, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::window::WindowBuilder;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Rect, Url, WebviewUrl, WebviewWindowBuilder,
    Window, WindowEvent, Wry,
};

use crate::config::AppConfig;
use crate::header;
use crate::icon;

pub const HEADER_HEIGHT: f64 = 36.0;

static POPUP_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct AppWindow {
    pub config: AppConfig,
    pub scripts: Vec<String>,
    pub profile_dir: PathBuf,
}

pub fn header_label(window: &str) -> String {
    format!("{window}:header")
}

pub fn content_label(window: &str) -> String {
    format!("{window}:content")
}

/// Settings shared by the content webview and popups (`WebviewBuilder` / `WebviewWindowBuilder`).
macro_rules! configure {
    ($builder:expr, $handle:expr, $app:expr) => {{
        let app: &Arc<AppWindow> = $app;
        let mut builder = $builder
            .data_directory(app.profile_dir.clone())
            .on_navigation({
                let app = app.clone();
                move |url| allow_navigation(&app.config, url)
            })
            .on_new_window({
                let app = app.clone();
                let handle = $handle.clone();
                move |url, features| new_window(&handle, &app, url, features)
            });
        if let Some(ua) = &app.config.user_agent {
            builder = builder.user_agent(ua);
        }
        for script in &app.scripts {
            builder = builder.initialization_script(script.clone());
        }
        builder
    }};
}

pub fn open(handle: &AppHandle, app: Arc<AppWindow>) -> Result<()> {
    let label = app.config.name.clone();
    let window = WindowBuilder::new(handle, &label)
        .title(app.config.title.as_deref().unwrap_or(&app.config.name))
        .inner_size(app.config.width, app.config.height)
        .decorations(false)
        .build()?;
    icon::set_app_id(&window, &app.config.name);
    header::register(handle, &label, app.clone());

    // Same data directory as the content: one browser environment per app.
    let header_view =
        WebviewBuilder::new(header_label(&label), WebviewUrl::App("header.html".into()))
            .data_directory(app.profile_dir.clone())
            .on_page_load(|webview, payload| {
                if payload.event() == PageLoadEvent::Finished {
                    header::push_state(webview.app_handle(), webview.window().label());
                }
            });
    let content_view = configure!(
        WebviewBuilder::new(
            content_label(&label),
            WebviewUrl::External(app.config.url.clone())
        ),
        handle,
        &app
    )
    .on_page_load(|webview, payload| {
        if payload.event() == PageLoadEvent::Finished {
            header::refresh_favicon(&webview);
        }
        let window = webview.window();
        header::set_page(
            webview.app_handle(),
            window.label(),
            None,
            Some(payload.url()),
        );
    })
    .on_document_title_changed({
        let fixed_title = app.config.title.is_some();
        move |webview, title| {
            let window = webview.window();
            if !fixed_title {
                let _ = window.set_title(&title);
            }
            // SPAs change the URL without a page load: refresh it along with the title.
            let url = webview.url().ok();
            header::set_page(
                webview.app_handle(),
                window.label(),
                Some(title),
                url.as_ref(),
            );
            // SPAs may swap the icon too (e.g. unread counters).
            header::refresh_favicon(&webview);
        }
    });

    let (header_rect, content_rect) = layout_rects(&window)?;
    let header_webview = window.add_child(header_view, header_rect.position, header_rect.size)?;
    #[cfg(target_os = "linux")]
    fix_header_height(&header_webview);
    #[cfg(not(target_os = "linux"))]
    let _ = header_webview;
    window.add_child(content_view, content_rect.position, content_rect.size)?;

    window.on_window_event({
        let window = window.clone();
        move |event| match event {
            WindowEvent::Resized(_) => {
                relayout(&window);
                header::push_state(window.app_handle(), window.label());
            }
            WindowEvent::Destroyed => header::unregister(window.app_handle(), window.label()),
            _ => {}
        }
    });
    Ok(())
}

fn layout_rects(window: &Window) -> tauri::Result<(Rect, Rect)> {
    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    let header_height = HEADER_HEIGHT.min(size.height);
    let header = Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(size.width, header_height).into(),
    };
    let content = Rect {
        position: LogicalPosition::new(0.0, header_height).into(),
        size: LogicalSize::new(size.width, size.height - header_height).into(),
    };
    Ok((header, content))
}

/// Linux packs child webviews in the window's `GtkBox`, ignoring bounds and sharing the
/// height equally: make the header a fixed-height, non-expanding child instead.
#[cfg(target_os = "linux")]
fn fix_header_height(header: &tauri::Webview) {
    let _ = header.with_webview(|webview| {
        use gtk::prelude::*;
        let widget = webview.inner();
        widget.set_size_request(-1, HEADER_HEIGHT as i32);
        if let Some(parent) = widget.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) {
            parent.set_child_packing(&widget, false, true, 0, gtk::PackType::Start);
        }
    });
}

/// Bounds of child webviews on Windows/macOS (no-op on Linux, see `fix_header_height`).
fn relayout(window: &Window) {
    let Ok((header_rect, content_rect)) = layout_rects(window) else {
        return;
    };
    let handle = window.app_handle();
    if let Some(webview) = handle.get_webview(&header_label(window.label())) {
        let _ = webview.set_bounds(header_rect);
    }
    if let Some(webview) = handle.get_webview(&content_label(window.label())) {
        let _ = webview.set_bounds(content_rect);
    }
}

fn is_internal_scheme(url: &Url) -> bool {
    matches!(url.scheme(), "about" | "data" | "blob" | "javascript")
}

fn is_allowed(config: &AppConfig, url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url
            .host_str()
            .is_some_and(|host| config.is_host_allowed(host))
}

pub fn open_external(url: &Url) {
    if let Err(e) = tauri_plugin_opener::open_url(url.as_str(), None::<&str>) {
        eprintln!("weboverlay: cannot open {url} externally: {e}");
    }
}

fn allow_navigation(config: &AppConfig, url: &Url) -> bool {
    if is_internal_scheme(url) || is_allowed(config, url) {
        return true;
    }
    open_external(url);
    false
}

/// `window.open` / `target="_blank"`: allowed hosts get a real popup (keeps `window.opener`,
/// needed for OAuth flows), everything else goes to the system browser.
fn new_window(
    handle: &AppHandle,
    app: &Arc<AppWindow>,
    url: Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<Wry> {
    if !is_allowed(&app.config, &url) && url.scheme() != "about" {
        open_external(&url);
        return NewWindowResponse::Deny;
    }
    let label = format!(
        "{}-popup-{}",
        app.config.name,
        POPUP_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let blank = WebviewUrl::External("about:blank".parse().expect("valid url"));
    let result = configure!(
        WebviewWindowBuilder::new(handle, &label, blank),
        handle,
        app
    )
    .window_features(features)
    .title(url.as_str())
    .on_document_title_changed(|window, title| {
        let _ = window.set_title(&title);
    })
    .build();
    match result {
        Ok(window) => {
            icon::set_app_id(&window.as_ref().window(), &app.config.name);
            NewWindowResponse::Create { window }
        }
        Err(e) => {
            eprintln!("weboverlay: cannot open popup for {url}: {e}");
            NewWindowResponse::Deny
        }
    }
}
