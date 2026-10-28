//! Window icon (taskbar / Alt+Tab) from the site favicon, plus a per-app taskbar identity on Windows
//! (own button, and a pin that relaunches the app with its config and favicon).
//!
//! The favicon is downloaded from Rust: the page and the header cannot read cross-origin image bytes.
//! Linux: only X11 window managers use the window icon; Wayland compositors use the `.desktop` file.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use base64::Engine;
use tauri::image::Image;
use tauri::{AppHandle, Manager, Url};

use crate::header;
use crate::window::AppWindow;

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
            {
                set_taskbar_icon(&window, &icon);
                if let Some(app) = header::app(&handle, &label) {
                    save_pin_icon(&window, &app, &icon);
                }
            }
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
    for pixel in bgra.as_chunks_mut::<4>().0 {
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

/// Taskbar identity of an app: `dev.blet.weboverlay.<config path hash>.<name>`, so apps with the
/// same name in different configs get their own button and pin.
#[cfg_attr(not(windows), allow(dead_code))]
fn app_id(app: &AppWindow) -> String {
    // Windows paths are case-insensitive. FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
    let path = app.config_path.to_string_lossy().to_lowercase();
    let hash = path.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let mut id = format!("dev.blet.weboverlay.{hash:016x}.{}", app.config.name);
    id.truncate(128); // AppUserModelID limit (the name is ASCII).
    id
}

/// Command run by a pinned taskbar button.
#[cfg_attr(not(windows), allow(dead_code))]
fn relaunch_command(exe: &Path, app: &AppWindow) -> String {
    // Windows paths cannot contain `"`, app names are `[A-Za-z0-9_-]+`.
    format!(
        "\"{}\" --config \"{}\" {}",
        exe.display(),
        app.config_path.display(),
        app.config.name
    )
}

/// Last favicon of an app as `.ico`: pinned buttons need an icon file.
#[cfg_attr(not(windows), allow(dead_code))]
fn pin_icon_path(app: &AppWindow) -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("weboverlay")
            .join("icons")
            .join(format!("{}.ico", app_id(app))),
    )
}

/// Sets string properties of the window property store (read by the taskbar).
#[cfg(windows)]
fn set_window_props(
    window: &tauri::Window,
    props: &[(&windows::Win32::Foundation::PROPERTYKEY, &str)],
) -> windows::core::Result<()> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, SHGetPropertyStoreForWindow,
    };

    let hwnd = window
        .hwnd()
        .map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
    unsafe {
        let store = SHGetPropertyStoreForWindow::<IPropertyStore>(HWND(hwnd.0))?;
        for (key, value) in props {
            store.SetValue(*key, &PROPVARIANT::from(*value))?;
        }
        store.Commit()
    }
}

/// Gives each app its own taskbar button/group instead of grouping all under the exe, and makes
/// a pinned button relaunch the app (same config) with its favicon instead of the bare exe.
#[cfg(windows)]
pub fn set_app_id(window: &tauri::Window, app: &AppWindow) {
    use windows::Win32::Storage::EnhancedStorage::{
        PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchCommand,
        PKEY_AppUserModel_RelaunchDisplayNameResource, PKEY_AppUserModel_RelaunchIconResource,
    };

    let id = app_id(app);
    let mut props = vec![(&PKEY_AppUserModel_ID, id.clone())];
    // The relaunch properties are only used together (display name required with the command).
    if let Some(exe) = crate::config::exe_path() {
        let icon = pin_icon_path(app)
            .filter(|path| path.is_file())
            .unwrap_or_else(|| exe.clone());
        props.push((
            &PKEY_AppUserModel_RelaunchCommand,
            relaunch_command(&exe, app),
        ));
        props.push((
            &PKEY_AppUserModel_RelaunchDisplayNameResource,
            app.config
                .title
                .clone()
                .unwrap_or_else(|| app.config.name.clone()),
        ));
        props.push((
            &PKEY_AppUserModel_RelaunchIconResource,
            format!("{},0", icon.display()),
        ));
    }
    let props: Vec<_> = props.iter().map(|(k, v)| (*k, v.as_str())).collect();
    if let Err(e) = set_window_props(window, &props) {
        eprintln!("weboverlay: cannot set taskbar id {id}: {e}");
    }
}

#[cfg(not(windows))]
pub fn set_app_id(_window: &tauri::Window, _app: &AppWindow) {}

/// Writes `icon` as the app pin icon and points the window relaunch icon at it.
#[cfg(windows)]
fn save_pin_icon(window: &tauri::Window, app: &AppWindow, icon: &Image<'_>) {
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_RelaunchIconResource;

    let Some(path) = pin_icon_path(app) else {
        return;
    };
    if let Err(e) = write_ico(&path, icon) {
        eprintln!("weboverlay: cannot write {}: {e}", path.display());
        return;
    }
    // The property store is COM: use it from the main thread, like `set_app_id`.
    let window = window.clone();
    let _ = window.clone().run_on_main_thread(move || {
        let value = format!("{},0", path.display());
        if let Err(e) = set_window_props(
            &window,
            &[(&PKEY_AppUserModel_RelaunchIconResource, &value)],
        ) {
            eprintln!("weboverlay: cannot set pin icon: {e}");
        }
    });
}

/// `.ico` holding `icon`, downscaled to the 256 px ICO limit; written atomically (the shell may
/// be reading the previous one).
#[cfg_attr(not(windows), allow(dead_code))]
fn write_ico(path: &Path, icon: &Image<'_>) -> anyhow::Result<()> {
    use anyhow::Context;

    let rgba = image::RgbaImage::from_raw(icon.width(), icon.height(), icon.rgba().to_vec())
        .context("invalid icon")?;
    let mut image = image::DynamicImage::ImageRgba8(rgba);
    if image.width() > 256 || image.height() > 256 {
        image = image.resize(256, 256, image::imageops::FilterType::Lanczos3);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("ico.tmp");
    image.save_with_format(&tmp, image::ImageFormat::Ico)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

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

    fn app_window(config: &str, name: &str) -> AppWindow {
        let toml = format!("[[app]]\nname = \"{name}\"\nurl = \"https://a.com\"");
        AppWindow {
            config: crate::config::Config::parse(&toml).unwrap().apps.remove(0),
            scripts: Vec::new(),
            profile_dir: PathBuf::new(),
            config_path: PathBuf::from(config),
        }
    }

    #[test]
    fn app_id_depends_on_config_path() {
        let a = app_id(&app_window("C:\\Users\\me\\config.toml", "mail"));
        assert_eq!(a, app_id(&app_window("c:\\users\\ME\\Config.toml", "mail")));
        assert_ne!(a, app_id(&app_window("D:\\config.toml", "mail")));
        assert!(a.starts_with("dev.blet.weboverlay.") && a.ends_with(".mail"));
        assert!(!a.contains(' '));
        assert!(app_id(&app_window("/c.toml", &"x".repeat(200))).len() <= 128);
    }

    #[test]
    fn relaunch_command_quotes_paths() {
        let app = app_window("C:\\My Configs\\config.toml", "mail");
        assert_eq!(
            relaunch_command(
                Path::new("C:\\Program Files\\WebOverlay\\weboverlay.exe"),
                &app
            ),
            r#""C:\Program Files\WebOverlay\weboverlay.exe" --config "C:\My Configs\config.toml" mail"#
        );
    }

    #[test]
    fn writes_ico() {
        let dir = std::env::temp_dir().join(format!("weboverlay-ico-{}", std::process::id()));
        let path = dir.join("a.ico");
        let icon = Image::new_owned(vec![255; 300 * 300 * 4], 300, 300);
        write_ico(&path, &icon).unwrap();
        let ico = image::open(&path).unwrap();
        assert_eq!((ico.width(), ico.height()), (256, 256));
        std::fs::remove_dir_all(dir).unwrap();
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
