//! Window icon (taskbar / Alt+Tab) from the site favicon, plus a per-app taskbar identity on Windows
//! (own button, and a pin that relaunches the app with its config and favicon). The last favicon of
//! each app is cached as a PNG for the launcher.
//!
//! The favicon is downloaded from Rust: the page and the header cannot read cross-origin image bytes.
//! Linux: only X11 window managers use the window icon; Wayland compositors use the `.desktop` file.
//! An app `icon` from the config replaces the favicon everywhere (header, window, taskbar, pin,
//! launcher).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use base64::Engine;
use tauri::image::Image;
use tauri::{AppHandle, Manager, Url};

use crate::config::AppConfig;
use crate::header;
use crate::window::AppWindow;

const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Raster size for SVG favicons (downscaled per platform when applied).
const SVG_SIZE: u32 = 128;
/// Max size of the launcher icon cache (32 px avatars, HiDPI).
const CACHE_SIZE: u32 = 64;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; WebOverlay)";
#[cfg(windows)]
const LAUNCHER_ID: &str = "dev.blet.weboverlay.launcher";

/// Downloads `favicon` in the background and sets it as the icon of window `label`.
/// Falls back to `/favicon.ico` for undecodable icons, then to the app icon.
pub fn update(handle: &AppHandle, label: &str, favicon: String, user_agent: Option<String>) {
    if header::app(handle, label).is_some_and(|app| app.icon.is_some()) {
        return; // the configured icon stays
    }
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
        let app = header::app(&handle, &label);
        if let (Some(image), Some(app)) = (&image, &app) {
            save_cached(&app_id(&app.config_path, &app.config.name), image);
        }
        let icon = image.or_else(|| handle.default_window_icon().cloned());
        if let Some(icon) = icon {
            #[cfg(windows)]
            if let Some(app) = &app {
                save_pin_icon(&window, app, &icon);
            }
            set_window_icon(&window, icon);
        }
    });
}

/// Image of the app `icon` key, read and decoded at startup so that a bad file fails early.
pub fn load_configured(app: &AppConfig) -> anyhow::Result<Option<Image<'static>>> {
    use anyhow::Context;

    let Some(path) = &app.icon else {
        return Ok(None);
    };
    let bytes = std::fs::read(path)
        .with_context(|| format!("app {:?}: cannot read icon {}", app.name, path.display()))?;
    let image = decode(&bytes).with_context(|| {
        format!(
            "app {:?}: cannot decode icon {} (PNG, ICO, JPEG, GIF, WebP, BMP or SVG)",
            app.name,
            path.display()
        )
    })?;
    Ok(Some(image))
}

/// Configured icon of a new app window: window and taskbar icon, launcher cache and, on Windows,
/// the pin icon file (read by `set_app_id`: call this first). No-op without `icon` key.
pub fn apply_configured(window: &tauri::Window, app: &AppWindow) {
    let Some(icon) = &app.icon else {
        return;
    };
    save_cached(&app_id(&app.config_path, &app.config.name), icon);
    #[cfg(windows)]
    if let Some(path) = pin_icon_path(app) {
        if let Err(e) = write_image(&path, icon, 256, image::ImageFormat::Ico) {
            eprintln!("weboverlay: cannot write {}: {e}", path.display());
        }
    }
    set_window_icon(window, icon.clone());
}

/// Window icon, plus the taskbar / Alt+Tab one on Windows.
pub fn set_window_icon(window: &tauri::Window, icon: Image<'_>) {
    #[cfg(windows)]
    set_taskbar_icon(window, &icon);
    let _ = window.set_icon(icon);
}

/// Image file `path` as a launcher card icon (`data:image/png` URL, like `cached`).
pub fn file_data_url(path: &Path) -> Option<String> {
    data_url(&decode(&std::fs::read(path).ok()?)?)
}

/// `icon` downscaled to `CACHE_SIZE`, as a `data:image/png` URL (launcher cards, header).
pub fn data_url(icon: &Image<'_>) -> Option<String> {
    let image = resized(icon, CACHE_SIZE).ok()?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
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
    // OS certificate store: corporate TLS inspection roots are trusted there, not in webpki-roots.
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
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

/// Identity of app `name` of the config at (absolute) `config_path`:
/// `dev.blet.weboverlay.<config path hash>.<name>`, so apps with the same name in different configs
/// get their own taskbar button, pin and cached icon.
fn app_id(config_path: &Path, name: &str) -> String {
    // Windows paths are case-insensitive. FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
    let path = config_path.to_string_lossy().to_lowercase();
    let hash = path.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let mut id = format!("dev.blet.weboverlay.{hash:016x}.{name}");
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

/// `<local data dir>/weboverlay/icons/<file>`.
fn icons_path(file: &str) -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("weboverlay")
            .join("icons")
            .join(file),
    )
}

/// Last favicon of an app as `.ico`: pinned buttons need an icon file.
#[cfg_attr(not(windows), allow(dead_code))]
fn pin_icon_path(app: &AppWindow) -> Option<PathBuf> {
    icons_path(&format!(
        "{}.ico",
        app_id(&app.config_path, &app.config.name)
    ))
}

/// Writes `icon` (downscaled to `CACHE_SIZE`) as the launcher icon of app `id` (errors are logged).
fn save_cached(id: &str, icon: &Image<'_>) {
    let Some(path) = icons_path(&format!("{id}.png")) else {
        return;
    };
    if let Err(e) = write_image(&path, icon, CACHE_SIZE, image::ImageFormat::Png) {
        eprintln!("weboverlay: cannot write {}: {e}", path.display());
    }
}

/// Cached favicon of app `name` of `config_path`, as a `data:image/png` URL.
pub fn cached(config_path: &Path, name: &str) -> Option<String> {
    let path = icons_path(&format!("{}.png", app_id(config_path, name)))?;
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Downloads `/favicon.ico` of `url` for an app never opened (the page favicon is only known once
/// it ran), caches it and returns it like `cached`. Blocking.
pub fn fetch_cached(
    config_path: &Path,
    name: &str,
    url: &Url,
    user_agent: Option<&str>,
) -> Option<String> {
    let favicon = url
        .join("/favicon.ico")
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))?;
    let image = decode(&fetch(favicon.as_str(), user_agent.unwrap_or(USER_AGENT))?)?;
    save_cached(&app_id(config_path, name), &image);
    cached(config_path, name)
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

    let id = app_id(&app.config_path, &app.config.name);
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

/// Taskbar identity of the launcher, so it doesn't share the exe group (whose cached icon can be
/// an app favicon); a pinned button relaunches the launcher. Call from the main thread (COM).
#[cfg(windows)]
pub fn set_launcher_id(window: &tauri::Window) {
    use windows::Win32::Storage::EnhancedStorage::{
        PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchCommand,
        PKEY_AppUserModel_RelaunchDisplayNameResource, PKEY_AppUserModel_RelaunchIconResource,
    };

    let mut props = vec![(&PKEY_AppUserModel_ID, LAUNCHER_ID.to_owned())];
    if let Some(exe) = crate::config::exe_path() {
        props.push((
            &PKEY_AppUserModel_RelaunchCommand,
            format!("\"{}\"", exe.display()),
        ));
        props.push((
            &PKEY_AppUserModel_RelaunchDisplayNameResource,
            "WebOverlay".to_owned(),
        ));
        props.push((
            &PKEY_AppUserModel_RelaunchIconResource,
            format!("{},0", exe.display()),
        ));
    }
    let props: Vec<_> = props.iter().map(|(k, v)| (*k, v.as_str())).collect();
    if let Err(e) = set_window_props(window, &props) {
        eprintln!("weboverlay: cannot set taskbar id {LAUNCHER_ID}: {e}");
    }
    set_default_taskbar_icon(window);
}

#[cfg(not(windows))]
pub fn set_launcher_id(_window: &tauri::Window) {}

/// The app icon as `ICON_BIG`: tao only sets `ICON_SMALL`, the taskbar then picks its own.
#[cfg(windows)]
pub fn set_default_taskbar_icon(window: &tauri::Window) {
    if let Some(icon) = window.app_handle().default_window_icon() {
        set_taskbar_icon(window, icon);
    }
}

#[cfg(not(windows))]
pub fn set_default_taskbar_icon(_window: &tauri::Window) {}

/// Writes `icon` as the app pin icon and points the window relaunch icon at it.
#[cfg(windows)]
fn save_pin_icon(window: &tauri::Window, app: &AppWindow, icon: &Image<'_>) {
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_RelaunchIconResource;

    let Some(path) = pin_icon_path(app) else {
        return;
    };
    if let Err(e) = write_image(&path, icon, 256, image::ImageFormat::Ico) {
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

/// `icon` downscaled to fit `max` px (256 = ICO limit), written atomically (the shell or the
/// launcher may be reading the previous one).
fn write_image(
    path: &Path,
    icon: &Image<'_>,
    max: u32,
    format: image::ImageFormat,
) -> anyhow::Result<()> {
    let image = resized(icon, max)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    image.save_with_format(&tmp, format)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// `icon` downscaled to fit `max` px.
fn resized(icon: &Image<'_>, max: u32) -> anyhow::Result<image::DynamicImage> {
    use anyhow::Context;

    let rgba = image::RgbaImage::from_raw(icon.width(), icon.height(), icon.rgba().to_vec())
        .context("invalid icon")?;
    let mut image = image::DynamicImage::ImageRgba8(rgba);
    if image.width() > max || image.height() > max {
        image = image.resize(max, max, image::imageops::FilterType::Lanczos3);
    }
    Ok(image)
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
            icon: None,
            profile_dir: PathBuf::new(),
            config_path: PathBuf::from(config),
        }
    }

    #[test]
    fn app_id_depends_on_config_path() {
        let a = app_id(Path::new("C:\\Users\\me\\config.toml"), "mail");
        assert_eq!(a, app_id(Path::new("c:\\users\\ME\\Config.toml"), "mail"));
        assert_ne!(a, app_id(Path::new("D:\\config.toml"), "mail"));
        assert!(a.starts_with("dev.blet.weboverlay.") && a.ends_with(".mail"));
        assert!(!a.contains(' '));
        assert!(app_id(Path::new("/c.toml"), &"x".repeat(200)).len() <= 128);
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
    fn writes_images() {
        let dir = std::env::temp_dir().join(format!("weboverlay-ico-{}", std::process::id()));
        let path = dir.join("a.ico");
        let icon = Image::new_owned(vec![255; 300 * 150 * 4], 300, 150);
        write_image(&path, &icon, 256, image::ImageFormat::Ico).unwrap();
        let ico = image::open(&path).unwrap();
        assert_eq!((ico.width(), ico.height()), (256, 128));
        let path = dir.join("a.png");
        write_image(&path, &icon, CACHE_SIZE, image::ImageFormat::Png).unwrap();
        let png = image::open(&path).unwrap();
        assert_eq!((png.width(), png.height()), (CACHE_SIZE, CACHE_SIZE / 2));
        assert!(!dir.join("a.png.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn configured_icon() {
        let dir = std::env::temp_dir().join(format!("weboverlay-cfg-icon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("icon.svg");
        std::fs::write(&svg, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="red"/></svg>"#).unwrap();
        let mut app = app_window("/c.toml", "mail").config;
        assert!(load_configured(&app).unwrap().is_none());
        app.icon = Some(svg.clone());
        let image = load_configured(&app).unwrap().unwrap();
        assert_eq!((image.width(), image.height()), (SVG_SIZE, SVG_SIZE));
        let url = file_data_url(&svg).unwrap();
        let png = decode_data_url(url.strip_prefix("data:").unwrap()).unwrap();
        let png = image::load_from_memory(&png).unwrap();
        assert_eq!((png.width(), png.height()), (CACHE_SIZE, CACHE_SIZE));

        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not an image").unwrap();
        app.icon = Some(bad);
        let error = format!("{:#}", load_configured(&app).unwrap_err());
        assert!(error.contains("cannot decode icon"), "{error}");
        app.icon = Some(dir.join("missing.png"));
        let error = format!("{:#}", load_configured(&app).unwrap_err());
        assert!(error.contains("cannot read icon"), "{error}");
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
