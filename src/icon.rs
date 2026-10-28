//! Window icon (taskbar / Alt+Tab) from the site favicon, plus a per-app taskbar identity on Windows.
//!
//! The favicon is downloaded from Rust: the page and the header cannot read cross-origin image bytes.
//! Linux: only X11 window managers use the window icon; Wayland compositors use the `.desktop` file.

use std::io::Read;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use base64::Engine;
use tauri::image::Image;
use tauri::{AppHandle, Manager, Url};

use crate::header;

const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Raster size for SVG favicons (downscaled per platform when applied).
const SVG_SIZE: u32 = 128;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; WebOverlay)";

/// Downloads `favicon` in the background and sets it as the icon of window `label`.
/// Falls back to `/favicon.ico` for undecodable icons, then to the app icon.
pub fn update(handle: &AppHandle, label: &str, favicon: String, user_agent: Option<String>) {
    let handle = handle.clone();
    let label = label.to_owned();
    thread::spawn(move || {
        let ua = user_agent.as_deref().unwrap_or(USER_AGENT);
        let image = candidates(&favicon)
            .into_iter()
            .find_map(|url| fetch(&url, ua).and_then(|bytes| decode(&bytes)));
        // The page may have changed while downloading: only apply the current favicon.
        if header::favicon(&handle, &label).as_deref() != Some(favicon.as_str()) {
            return;
        }
        let Some(window) = handle.get_window(&label) else {
            return;
        };
        let icon = image.or_else(|| handle.default_window_icon().cloned());
        if let Some(icon) = icon {
            #[cfg(windows)]
            set_taskbar_icon(&window, &icon);
            let _ = window.set_icon(icon);
        }
    });
}

fn candidates(favicon: &str) -> Vec<String> {
    let mut urls = Vec::new();
    if !favicon.is_empty() {
        urls.push(favicon.to_owned());
    }
    if let Some(fallback) = Url::parse(favicon)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .and_then(|u| u.join("/favicon.ico").ok())
    {
        if fallback.as_str() != favicon {
            urls.push(fallback.to_string());
        }
    }
    urls
}

fn fetch(url: &str, user_agent: &str) -> Option<Vec<u8>> {
    if let Some(data) = url.strip_prefix("data:") {
        return decode_data_url(data);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let response = agent
        .get(url)
        .header("User-Agent", user_agent)
        .call()
        .ok()?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// Payload of a data URL without the `data:` prefix (`image/png;base64,AAAA`, `image/svg+xml,<svg…>`).
fn decode_data_url(data: &str) -> Option<Vec<u8>> {
    let (meta, payload) = data.split_once(',')?;
    if meta.ends_with(";base64") {
        let payload: String = payload.split_whitespace().collect();
        return base64::engine::general_purpose::STANDARD
            .decode(payload)
            .ok();
    }
    Some(percent_encoding::percent_decode_str(payload).collect())
}

/// Raster formats through `tauri::image`, else SVG through resvg.
fn decode(bytes: &[u8]) -> Option<Image<'static>> {
    Image::from_bytes(bytes).ok().or_else(|| render_svg(bytes))
}

fn render_svg(bytes: &[u8]) -> Option<Image<'static>> {
    use resvg::{tiny_skia, usvg};

    // Loading system fonts is slow: once per process (emoji favicons are `<text>` SVGs).
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    });
    let mut options = usvg::Options {
        fontdb: fonts.clone(),
        ..Default::default()
    };
    // Untrusted input: never read files referenced by `<image href>`.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);

    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let size = tree.size();
    let scale = SVG_SIZE as f32 / size.width().max(size.height());
    let mut pixmap = tiny_skia::Pixmap::new(SVG_SIZE, SVG_SIZE)?;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(
        (SVG_SIZE as f32 - size.width() * scale) / 2.0,
        (SVG_SIZE as f32 - size.height() * scale) / 2.0,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    if pixmap.pixels().iter().all(|p| p.alpha() == 0) {
        return None; // Nothing rendered (e.g. missing font): keep falling back.
    }
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Some(Image::new_owned(rgba, SVG_SIZE, SVG_SIZE))
}

/// Tauri's `set_icon` only sets `ICON_SMALL` (title bar); the taskbar and Alt+Tab use `ICON_BIG`.
#[cfg(windows)]
fn set_taskbar_icon(window: &tauri::Window, icon: &Image<'_>) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIcon, DestroyIcon, GetSystemMetrics, SendMessageW, HICON, ICON_BIG, SM_CXICON,
        WM_SETICON,
    };

    /// HICONs we created, by HWND, destroyed when replaced (the initial one belongs to tao).
    static CREATED: Mutex<Option<HashMap<isize, isize>>> = Mutex::new(None);

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let size = (unsafe { GetSystemMetrics(SM_CXICON) }).max(32) as u32;
    let Some(rgba) = image::RgbaImage::from_raw(icon.width(), icon.height(), icon.rgba().to_vec())
    else {
        return;
    };
    let rgba = if rgba.width() > size || rgba.height() > size {
        image::imageops::resize(&rgba, size, size, image::imageops::FilterType::Lanczos3)
    } else {
        rgba
    };
    let (width, height) = rgba.dimensions();
    let mut bgra = rgba.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    // 32 bpp with alpha: the AND mask is ignored but must be provided.
    let and_mask = vec![0u8; (width * height) as usize];
    let Ok(hicon) = (unsafe {
        CreateIcon(
            None,
            width as i32,
            height as i32,
            1,
            32,
            and_mask.as_ptr(),
            bgra.as_ptr(),
        )
    }) else {
        return;
    };
    unsafe {
        SendMessageW(
            HWND(hwnd.0),
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(hicon.0 as isize)),
        );
    }
    let mut created = CREATED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(previous) = created
        .get_or_insert_with(HashMap::new)
        .insert(hwnd.0 as isize, hicon.0 as isize)
    {
        let _ = unsafe { DestroyIcon(HICON(previous as _)) };
    }
}

/// Gives each app its own taskbar button/group (and pin) instead of grouping all under the exe.
#[cfg(windows)]
pub fn set_app_id(window: &tauri::Window, app_name: &str) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, SHGetPropertyStoreForWindow,
    };

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let id = format!("dev.blet.weboverlay.{app_name}");
    let result = unsafe {
        SHGetPropertyStoreForWindow::<IPropertyStore>(HWND(hwnd.0)).and_then(|store| {
            store.SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(id.as_str()))?;
            store.Commit()
        })
    };
    if let Err(e) = result {
        eprintln!("weboverlay: cannot set taskbar id: {e}");
    }
}

#[cfg(not(windows))]
pub fn set_app_id(_window: &tauri::Window, _app_name: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_svg_shapes() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="8" fill="red"/></svg>"#;
        let image = decode(svg).expect("svg renders");
        assert_eq!((image.width(), image.height()), (SVG_SIZE, SVG_SIZE));
        // Top half red and opaque, bottom half transparent.
        assert_eq!(&image.rgba()[..4], &[255, 0, 0, 255]);
        assert_eq!(image.rgba()[image.rgba().len() - 1], 0);
    }

    #[test]
    fn emoji_svg_favicon() {
        // Same shape as news.mblet.fr. Needs a system emoji font; writes the render for inspection.
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'><text y='13' font-size='13'>&#128240;</text></svg>";
        match decode(svg.as_bytes()) {
            Some(image) => {
                if let Ok(path) = std::env::var("WEBOVERLAY_ICON_OUT") {
                    image::RgbaImage::from_raw(
                        image.width(),
                        image.height(),
                        image.rgba().to_vec(),
                    )
                    .unwrap()
                    .save(path)
                    .unwrap();
                }
            }
            None => eprintln!("no emoji font available: emoji favicon not rendered"),
        }
    }

    #[test]
    fn candidates_add_favicon_ico_fallback() {
        assert_eq!(
            candidates("https://a.com/static/icon.svg"),
            ["https://a.com/static/icon.svg", "https://a.com/favicon.ico"]
        );
        assert_eq!(
            candidates("https://a.com/favicon.ico"),
            ["https://a.com/favicon.ico"]
        );
        assert_eq!(
            candidates("data:image/png;base64,AA=="),
            ["data:image/png;base64,AA=="]
        );
        assert!(candidates("").is_empty());
    }

    #[test]
    fn data_urls() {
        assert_eq!(
            decode_data_url("image/png;base64,AQID"),
            Some(vec![1, 2, 3])
        );
        assert_eq!(
            decode_data_url("image/svg+xml,%3Csvg%3E").as_deref(),
            Some(&b"<svg>"[..])
        );
        assert_eq!(
            decode_data_url("image/svg+xml,<svg x='1'/>").as_deref(),
            Some(&b"<svg x='1'/>"[..])
        );
    }
}
