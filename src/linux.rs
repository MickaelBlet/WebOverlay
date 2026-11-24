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

use std::path::Path;

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
