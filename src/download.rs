//! Downloads of the content and popups: the webview saves the file where it chooses (the
//! downloads folder) while a native "save as" dialog asks for the real destination; the file is
//! moved there once both are known, or deleted when the dialog is cancelled.
//!
//! The dialog is asynchronous (built on the main thread like the launcher's, awaited from a
//! thread): the download handler must answer at once, and a modal loop inside a WebKitGTK /
//! WebView2 event is not safe. It opens on `Requested` (WebView2, WebKitGTK when it asks for a
//! destination), else on `Finished`: WebKitGTK 2.52 picks the destination of `<a download>`
//! links itself, so wry never reports them as requested.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tauri::webview::DownloadEvent;
use tauri::{Runtime, Webview, WebviewWindow};

struct Pending {
    id: u64,
    url: String,
    /// Where the webview saves the file.
    path: PathBuf,
    /// Answer of the dialog: `Some(None)` = cancelled.
    target: Option<Option<PathBuf>>,
    /// Download result: `Some(false)` = failed, nothing to move.
    done: Option<bool>,
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn pending() -> std::sync::MutexGuard<'static, Vec<Pending>> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// `on_download` handler of the content webview and popups.
pub fn handle<R: Runtime>(webview: Webview<R>, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            pending().push(Pending {
                id,
                url: url.to_string(),
                path: destination.clone(),
                target: None,
                done: None,
            });
            ask_target(&webview, id, destination);
        }
        DownloadEvent::Finished { url, path, success } => {
            let url = url.to_string();
            let saved = path.filter(|_| success);
            let mut list = pending();
            // Linux runs the handler of every webview sharing the web context (the content and
            // its popups) in the same signal emission: only the first call finds an unfinished
            // download, the dialog cannot have answered in between.
            let found = saved
                .as_ref()
                .and_then(|saved| list.iter().position(|p| same_file(saved, &p.path)))
                .or_else(|| list.iter().position(|p| p.done.is_none() && p.url == url));
            match (found, saved) {
                (Some(index), saved) if list[index].done.is_none() => {
                    let p = &mut list[index];
                    if let Some(saved) = saved {
                        p.path = saved; // final name (WebView2 may append a number)
                    }
                    p.done = Some(success);
                    let id = p.id;
                    drop(list);
                    settle(id);
                }
                // Saved without `Requested`: ask now where it goes.
                (None, Some(saved)) => {
                    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                    list.push(Pending {
                        id,
                        url,
                        path: saved.clone(),
                        target: None,
                        done: Some(true),
                    });
                    drop(list);
                    ask_target(&webview, id, &saved);
                }
                _ => {}
            }
        }
        _ => {}
    }
    true
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || fs::canonicalize(a)
            .ok()
            .is_some_and(|a| fs::canonicalize(b).ok() == Some(a))
}

/// Shows the "save as" dialog for download `id`, suggesting `path`.
fn ask_target<R: Runtime>(webview: &Webview<R>, id: u64, path: &Path) {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Save file");
    if let Some(name) = path.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy());
    }
    if let Some(dir) = path.parent() {
        dialog = dialog.set_directory(dir);
    }
    // Download events run on the main thread: the dialog can be built right away.
    let picked = dialog.set_parent(&webview.window()).save_file();
    std::thread::spawn(move || {
        let target = tauri::async_runtime::block_on(picked).map(|f| f.path().to_path_buf());
        if let Some(p) = pending().iter_mut().find(|p| p.id == id) {
            p.target = Some(target);
        }
        settle(id);
    });
}

/// Moves (or deletes) the file of download `id` once the download and the dialog are both done.
fn settle(id: u64) {
    let (target, saved) = {
        let mut list = pending();
        let Some(index) = list.iter().position(|p| p.id == id) else {
            return;
        };
        let (Some(target), Some(success)) = (&list[index].target, list[index].done) else {
            return;
        };
        let target = target.clone();
        let p = list.remove(index);
        if !success {
            return;
        }
        (target, p.path)
    };
    let result = match target {
        Some(target) if same_file(&saved, &target) => Ok(()),
        Some(target) => move_file(&saved, &target),
        None => fs::remove_file(&saved),
    };
    if let Err(e) = result {
        eprintln!("weboverlay: download {}: {e}", saved.display());
    }
}

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    // `rename` fails across filesystems: copy then delete instead.
    fs::rename(from, to).or_else(|_| {
        fs::copy(from, to)?;
        fs::remove_file(from)
    })
}

/// Linux download fixes for a content webview or a popup (`popup` = its window).
///
/// WebKitGTK only downloads `Content-Disposition: attachment` responses and ignores the other
/// types it cannot show (a link to a `.zip`, a `.exe`...): download them like browsers do
/// (WebView2 already does). A popup opened only for a download (`target="_blank"` link) stays
/// empty: close it like browsers do.
pub fn setup<R: Runtime>(webview: &Webview<R>, popup: Option<WebviewWindow<R>>) {
    #[cfg(target_os = "linux")]
    let _ = webview.with_webview(move |webview| {
        use gtk::prelude::WidgetExt;
        use webkit2gtk::glib::{self, Cast};
        use webkit2gtk::{
            DownloadExt, PolicyDecisionExt, PolicyDecisionType, ResponsePolicyDecision,
            ResponsePolicyDecisionExt, WebContextExt, WebViewExt,
        };
        let view = webview.inner();
        // Connected after wry's navigation handler, which leaves response decisions alone.
        view.connect_decide_policy(|_, decision, decision_type| {
            let unsupported = decision_type == PolicyDecisionType::Response
                && decision
                    .downcast_ref::<ResponsePolicyDecision>()
                    .is_some_and(|r| {
                        r.is_main_frame_main_resource() && !r.is_mime_type_supported()
                    });
            if unsupported {
                decision.download();
            }
            unsupported
        });
        let (Some(popup), Some(context)) = (popup, view.context()) else {
            return;
        };
        // The web context is shared by the app's webviews: keep the downloads of this popup.
        let handler = context.connect_download_started({
            let view = view.clone();
            move |_, download| {
                let blank = view.uri().is_none_or(|uri| uri.is_empty());
                if blank && download.web_view().as_ref() == Some(&view) {
                    let popup = popup.clone();
                    // Not inside the signal: the download keeps going without its webview.
                    glib::idle_add_local_once(move || {
                        let _ = popup.close();
                    });
                }
            }
        });
        let handler = std::cell::Cell::new(Some(handler));
        view.connect_destroy(move |_| {
            if let Some(handler) = handler.take() {
                glib::signal::signal_handler_disconnect(&context, handler);
            }
        });
    });
    #[cfg(not(target_os = "linux"))]
    let _ = (webview, popup);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_file_replaces_target() {
        let dir = std::env::temp_dir().join(format!("weboverlay-dl-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("a.txt"), dir.join("b.txt"));
        fs::write(&from, "new").unwrap();
        fs::write(&to, "old").unwrap();
        move_file(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "new");
        assert!(same_file(&to, &dir.join(".").join("b.txt")));
        fs::remove_dir_all(&dir).unwrap();
    }
}
