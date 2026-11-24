//! Linux process environment, set at the very start of `main` (before GTK and before any thread,
//! WebKit's helper processes inherit it).
//!
//! Without a usable GPU (virtual machine without 3D, no render node) Mesa is sent straight to its
//! software rasterizer, and WebKit's Skia paints the page tiles on the CPU
//! (`WEBKIT_SKIA_ENABLE_CPU_RENDERING`, one painting thread per core, max 8): Skia's GL painting
//! on llvmpipe is the slow part, llvmpipe is then only used to composite the layers (about 3x
//! faster on a repaint-heavy page under Xvfb, WebKitGTK 2.52). Accelerated compositing itself is
//! kept: WebKitGTK's non-composited mode (`WEBKIT_DISABLE_COMPOSITING_MODE`, and also
//! `WEBKIT_DISABLE_DMABUF_RENDERER`, which turns the policy to `never`, see `webkit://gpu`) is
//! faster still, but the web process aborts in it as soon as a page needs a compositing layer
//! (`FrameRenderer::graphicsLayerFactory` release assert, WebKit bugs 265460 / 306993, e.g.
//! Artifactory), so it is left to the user. `WEBOVERLAY_GPU=1` keeps the GPU path, `=0` forces
//! the software one; variables already set by the user are never overridden.
//!
//! In software mode `setup_webviews` also turns on WebKit's `UseDamagingInformationForCompositing`
//! feature in every webview: the compositor then only redraws the changed region instead of the
//! whole view, so a page with a never-ending animation (a 24 px CSS spinner) no longer keeps
//! llvmpipe busy (about 75 % of a core → 15 % under Xvfb). WebKit reads it when the page is
//! created, before the first load: too late in `with_webview`, so a GTK `parent-set` emission
//! hook sets it when wry packs the new `WebKitWebView` in its container, just before `load_uri`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set by `setup_env`: Mesa software rendering is in use.
static SOFTWARE: AtomicBool = AtomicBool::new(false);

/// DRM drivers of virtual or display-only GPUs: no (or no usable) 3D acceleration.
const SOFTWARE_DRIVERS: &[&str] = &[
    "vmwgfx",
    "vboxvideo",
    "qxl",
    "bochs",
    "bochs-drm",
    "cirrus",
    "cirrus-qemu",
    "simpledrm",
    "simple-framebuffer",
    "hyperv_drm",
    "vkms",
    "udl",
    "ast",
    "mgag200",
];

pub fn setup_env() {
    // The AppImage's own GTK cannot load the host's modules (`pk-gtk-module`,
    // `canberra-gtk-module`...): each one only prints "Failed to load module".
    if std::env::var_os("APPIMAGE").is_some() {
        std::env::remove_var("GTK_MODULES");
    }
    let software = match std::env::var("WEBOVERLAY_GPU").as_deref() {
        Ok("1") => false,
        Ok("0") => true,
        _ => !has_gpu(Path::new("/")),
    };
    SOFTWARE.store(software, Ordering::Relaxed);
    if software {
        // Mesa goes straight to llvmpipe instead of probing the virtual GPU first
        // ("VMware: No 3D enabled", "failed to create dri2 screen").
        set_default("LIBGL_ALWAYS_SOFTWARE", "1");
        // Skia rasterizes on the CPU (multithreaded) instead of through llvmpipe; ignored by
        // WebKitGTK before 2.46 (Cairo).
        set_default("WEBKIT_SKIA_ENABLE_CPU_RENDERING", "1");
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        set_default("WEBKIT_SKIA_CPU_PAINTING_THREADS", &threads.to_string());
    }
}

/// Software mode: enables `UseDamagingInformationForCompositing` in every `WebKitWebView` created
/// from now on (GTK main thread, GTK initialized: Tauri's `setup`).
pub fn setup_webviews() {
    use gtk::glib::gobject_ffi;
    use gtk::glib::translate::IntoGlib;
    use gtk::glib::StaticType;

    if !SOFTWARE.load(Ordering::Relaxed) {
        return;
    }
    let Some(features) = Features::load() else {
        return; // WebKitGTK before 2.42: no feature API.
    };
    if FEATURES.set(features).is_err() {
        return; // Already installed.
    }
    unsafe extern "C" fn hook(
        _hint: *mut gobject_ffi::GSignalInvocationHint,
        n_params: u32,
        params: *const gobject_ffi::GValue,
        _data: gtk::glib::ffi::gpointer,
    ) -> gtk::glib::ffi::gboolean {
        if n_params > 0 {
            let instance =
                gobject_ffi::g_value_get_object(params) as *mut gobject_ffi::GTypeInstance;
            let web_view = webkit2gtk::ffi::webkit_web_view_get_type();
            if !instance.is_null()
                && gobject_ffi::g_type_check_instance_is_a(instance, web_view) != 0
            {
                if let Some(features) = FEATURES.get() {
                    features.enable(instance.cast(), c"UseDamagingInformationForCompositing");
                }
            }
        }
        gtk::glib::ffi::GTRUE // Keep the hook.
    }
    unsafe {
        let widget = gtk::Widget::static_type().into_glib();
        let class = gobject_ffi::g_type_class_ref(widget);
        let signal = gobject_ffi::g_signal_lookup(c"parent-set".as_ptr(), widget);
        if signal != 0 {
            gobject_ffi::g_signal_add_emission_hook(
                signal,
                0,
                Some(hook),
                std::ptr::null_mut(),
                None,
            );
        }
        gobject_ffi::g_type_class_unref(class);
    }
}

static FEATURES: std::sync::OnceLock<Features> = std::sync::OnceLock::new();

/// WebKitGTK 2.42+ feature API, looked up at run time: linking it would stop the app from starting
/// with an older WebKitGTK. `WebKitFeatureList` / `WebKitFeature` / `WebKitSettings` /
/// `WebKitWebView` pointers as `Ptr`, `gboolean` as `i32`, `gsize` as `usize`.
struct Features {
    all: unsafe extern "C" fn() -> Ptr,
    length: unsafe extern "C" fn(Ptr) -> usize,
    get: unsafe extern "C" fn(Ptr, usize) -> Ptr,
    identifier: unsafe extern "C" fn(Ptr) -> *const std::ffi::c_char,
    unref: unsafe extern "C" fn(Ptr),
    settings: unsafe extern "C" fn(Ptr) -> Ptr,
    set_enabled: unsafe extern "C" fn(Ptr, Ptr, i32),
}

type Ptr = *mut std::ffi::c_void;

impl Features {
    fn load() -> Option<Self> {
        /// SAFETY: `F` must be the `extern "C" fn` type of symbol `name`.
        unsafe fn symbol<F: Copy>(name: &std::ffi::CStr) -> Option<F> {
            let ptr = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
            (!ptr.is_null()).then(|| std::mem::transmute_copy::<Ptr, F>(&ptr))
        }
        // SAFETY: signatures of the WebKitGTK headers (`WebKitFeature.h`, `WebKitSettings.h`).
        unsafe {
            Some(Self {
                all: symbol(c"webkit_settings_get_all_features")?,
                length: symbol(c"webkit_feature_list_get_length")?,
                get: symbol(c"webkit_feature_list_get")?,
                identifier: symbol(c"webkit_feature_get_identifier")?,
                unref: symbol(c"webkit_feature_list_unref")?,
                settings: symbol(c"webkit_web_view_get_settings")?,
                set_enabled: symbol(c"webkit_settings_set_feature_enabled")?,
            })
        }
    }

    /// Enables feature `id` (no-op when this WebKit doesn't have it) in the settings of `view`.
    unsafe fn enable(&self, view: Ptr, id: &std::ffi::CStr) {
        let settings = (self.settings)(view);
        let list = (self.all)();
        if settings.is_null() || list.is_null() {
            return;
        }
        for i in 0..(self.length)(list) {
            let feature = (self.get)(list, i);
            let name = (self.identifier)(feature);
            if !name.is_null() && std::ffi::CStr::from_ptr(name) == id {
                (self.set_enabled)(settings, feature, 1);
                break;
            }
        }
        (self.unref)(list); // `transfer full`
    }
}

fn set_default(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

/// Whether a render node of a 3D-capable driver exists (`root` = `/`, other for tests).
/// WSLg has no render node but a GPU through `/dev/dxg`.
fn has_gpu(root: &Path) -> bool {
    if root.join("dev/dxg").exists() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(root.join("sys/class/drm")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_name().to_string_lossy().starts_with("renderD")
            && std::fs::read_link(entry.path().join("device/driver"))
                .ok()
                .and_then(|driver| driver.file_name().map(|n| n.to_string_lossy().into_owned()))
                .is_some_and(|driver| !SOFTWARE_DRIVERS.contains(&driver.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_node(root: &Path, node: &str, driver: &str) {
        let device = root.join("sys/class/drm").join(node).join("device");
        std::fs::create_dir_all(&device).unwrap();
        let target = root.join("sys/bus/pci/drivers").join(driver);
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, device.join("driver")).unwrap();
    }

    #[test]
    fn gpu_detection() {
        let dir = std::env::temp_dir().join(format!("weboverlay-gpu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (none, vm, real) = (dir.join("none"), dir.join("vm"), dir.join("real"));
        std::fs::create_dir_all(none.join("sys/class/drm/card0")).unwrap();
        render_node(&vm, "renderD128", "vmwgfx");
        render_node(&real, "renderD128", "vmwgfx");
        render_node(&real, "renderD129", "i915");
        assert!(!has_gpu(&none));
        assert!(!has_gpu(&vm));
        assert!(has_gpu(&real));
        std::fs::create_dir_all(none.join("dev/dxg")).unwrap();
        assert!(has_gpu(&none));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
