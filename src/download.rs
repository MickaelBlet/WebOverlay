//! Downloads of the content and popups, tracked from the native webview (WebKitGTK
//! `WebKitDownload`, WebView2 `DownloadOperation`: Tauri's download events carry no progress and
//! can't cancel) and listed in the downloads window (`dist/downloads.html`).
//!
//! The webview saves the file in the downloads folder while a native "save as" dialog asks for
//! the real destination; the file is moved there once both are known, or the download is
//! cancelled (its file deleted) when the dialog is. The dialog is asynchronous (built on the main
//! thread like the launcher's, awaited from a thread): the download events must return at once,
//! and a modal loop inside a WebKitGTK / WebView2 event is not safe.
//!
//! Links outside `allowed_hosts` are probed first (`probe`): downloads stay in the app, the rest
//! goes to the system browser.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    AppHandle, Manager, Url, Webview, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window,
};

use crate::header;
use crate::icon;
use crate::window::{show_when_loaded, theme_color, PAGE_BG};

/// Contains `:`, so it can't clash with an app window label (`[A-Za-z0-9_-]+`).
pub const DOWNLOADS_LABEL: &str = "weboverlay:downloads";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum State {
    Running,
    Done,
    Failed,
    Cancelled,
}

struct Item {
    id: u64,
    url: String,
    /// Where the webview saves the file, then where it was moved.
    path: PathBuf,
    /// Proposed by the dialog (`path` without the `.part` of WebView2 downloads).
    suggested: PathBuf,
    /// Answer of the dialog: `Some(None)` = cancelled.
    target: Option<Option<PathBuf>>,
    /// The file was moved to `target` (or is being moved).
    settled: bool,
    /// `open_downloads` type: no dialog, opened with its default app once saved.
    open: bool,
    received: u64,
    total: Option<u64>,
    state: State,
}

/// What the downloads page shows of an item.
#[derive(Serialize)]
pub struct View {
    id: u64,
    name: String,
    url: String,
    path: PathBuf,
    received: u64,
    total: Option<u64>,
    state: State,
    /// The "save as" dialog is still open.
    asking: bool,
}

static ITEMS: Mutex<Vec<Item>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(0);
/// A download was started in this process: the header shows its downloads button.
static STARTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Main thread: cancels native download `id` (WebKit / COM objects are not `Send`).
    static CANCEL: RefCell<HashMap<u64, Box<dyn Fn()>>> = RefCell::new(HashMap::new());
}

fn items() -> MutexGuard<'static, Vec<Item>> {
    ITEMS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Some download was started: the header shows its downloads button.
pub fn started() -> bool {
    STARTED.load(Ordering::Relaxed)
}

/// Main thread: download of `url` saved at `path` by a webview of `parent` (an app window),
/// `suggested` to the dialog, or kept there and opened for the app's `open_downloads` types.
fn start(
    parent: &Window,
    url: String,
    path: PathBuf,
    suggested: PathBuf,
    total: Option<u64>,
    cancel: Box<dyn Fn()>,
) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let name = suggested.file_name().unwrap_or_default().to_string_lossy();
    let open = header::app(parent.app_handle(), parent.label())
        .is_some_and(|app| app.config.opens_download(&name));
    if !open {
        ask_target(parent, id, &suggested);
    }
    items().push(Item {
        id,
        url,
        path,
        target: open.then(|| Some(suggested.clone())),
        suggested,
        settled: false,
        open,
        received: 0,
        total,
        state: State::Running,
    });
    CANCEL.with(|c| c.borrow_mut().insert(id, cancel));
    if !STARTED.swap(true, Ordering::Relaxed) {
        header::push_all(parent.app_handle());
    }
    id
}

fn progress(id: u64, received: u64, total: Option<u64>) {
    if let Some(item) = items().iter_mut().find(|i| i.id == id) {
        item.received = received;
        if total.is_some() {
            item.total = total;
        }
    }
}

/// Main thread: end of native download `id` (`path` = final file when it succeeded).
fn finish(id: u64, path: Option<PathBuf>, success: bool) {
    CANCEL.with(|c| c.borrow_mut().remove(&id));
    if let Some(item) = items().iter_mut().find(|i| i.id == id) {
        if item.state == State::Running {
            item.state = if success { State::Done } else { State::Failed };
        }
        if let Some(path) = path.filter(|_| success) {
            item.path = path; // final name (WebView2 may append a number)
        }
        if success {
            let size = fs::metadata(&item.path).map_or(item.received, |m| m.len());
            item.received = size;
            item.total = Some(size);
        }
    }
    settle(id);
}

/// Main thread: cancels download `id` if still running.
fn cancel(id: u64) {
    if let Some(item) = items().iter_mut().find(|i| i.id == id) {
        if item.state == State::Running {
            item.state = State::Cancelled;
        }
    }
    // Out of the map first: the native cancel may call `finish` synchronously.
    if let Some(cancel) = CANCEL.with(|c| c.borrow_mut().remove(&id)) {
        cancel();
    }
    settle(id);
}

/// Shows the "save as" dialog for download `id`, suggesting `path`.
fn ask_target(parent: &Window, id: u64, path: &Path) {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Save file");
    if let Some(name) = path.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy());
    }
    if let Some(dir) = path.parent() {
        dialog = dialog.set_directory(dir);
    }
    // Download events run on the main thread: the dialog can be built right away.
    let picked = dialog.set_parent(parent).save_file();
    let handle = parent.app_handle().clone();
    std::thread::spawn(move || {
        let target = tauri::async_runtime::block_on(picked).map(|f| f.path().to_path_buf());
        let chosen = target.is_some();
        let running = match items().iter_mut().find(|i| i.id == id) {
            Some(item) => {
                item.target = Some(target);
                item.state == State::Running
            }
            None => return,
        };
        if chosen {
            if let Err(e) = open(&handle) {
                eprintln!("weboverlay: cannot open the downloads window: {e}");
            }
            settle(id);
        } else if running {
            let _ = handle.run_on_main_thread(move || cancel(id));
        } else {
            settle(id);
        }
    });
}

/// Moves (or deletes) the file of download `id` once the download and the dialog are both done.
fn settle(id: u64) {
    let (target, saved) = {
        let mut list = items();
        let Some(index) = list.iter().position(|i| i.id == id) else {
            return;
        };
        let item = &mut list[index];
        let Some(target) = item.target.clone() else {
            return;
        };
        if item.state == State::Running || item.settled {
            return;
        }
        match target {
            Some(target) if item.state == State::Done => {
                item.settled = true;
                (target, item.path.clone())
            }
            Some(_) => return, // failed or cancelled: shown as such
            None => {
                let item = list.remove(index);
                if item.state == State::Done {
                    if let Err(e) = fs::remove_file(&item.path) {
                        eprintln!("weboverlay: download {}: {e}", item.path.display());
                    }
                }
                return;
            }
        }
    };
    let result = if same_file(&saved, &target) {
        Ok(())
    } else {
        move_file(&saved, &target)
    };
    let mut open = false;
    if let Some(item) = items().iter_mut().find(|i| i.id == id) {
        match result {
            Ok(()) => {
                item.path = target.clone();
                open = item.open;
            }
            Err(e) => {
                eprintln!("weboverlay: download {}: {e}", saved.display());
                item.state = State::Failed;
            }
        }
    }
    if open {
        if let Err(e) = tauri_plugin_opener::open_path(&target, None::<&str>) {
            eprintln!("weboverlay: cannot open {}: {e}", target.display());
        }
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || fs::canonicalize(a)
            .ok()
            .is_some_and(|a| fs::canonicalize(b).ok() == Some(a))
}

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    // `rename` fails across filesystems: copy then delete instead.
    fs::rename(from, to).or_else(|_| {
        fs::copy(from, to)?;
        fs::remove_file(from)
    })
}

/// Native download tracking of a content webview or a popup (`popup` = its window); `parent`
/// = the app window (owner of the dialogs: a popup may close before they open).
pub fn setup(webview: &Webview, parent: Window, popup: Option<WebviewWindow>) {
    #[cfg(target_os = "linux")]
    let _ = webview.with_webview(move |webview| linux::setup(webview.inner(), parent, popup));
    #[cfg(windows)]
    {
        let _ = popup;
        let _ = webview.with_webview(move |webview| webview2::setup(webview.controller(), parent));
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = (webview, parent, popup);
}

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;
    use std::rc::Rc;

    use gtk::prelude::WidgetExt;
    use tauri::{WebviewWindow, Window};
    use webkit2gtk::glib::{self, Cast};
    use webkit2gtk::{
        Download, DownloadExt, PolicyDecisionExt, PolicyDecisionType, ResponsePolicyDecision,
        ResponsePolicyDecisionExt, URIRequestExt, URIResponseExt, WebContextExt, WebView,
        WebViewExt,
    };

    /// WebKitGTK only downloads `Content-Disposition: attachment` responses and ignores the
    /// other types it cannot show (a link to a `.zip`, a `.exe`...): download them like browsers
    /// do (WebView2 already does). A popup opened only for a download (`target="_blank"` link)
    /// stays empty: close it like browsers do.
    pub fn setup(view: WebView, parent: Window, popup: Option<WebviewWindow>) {
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
        let Some(context) = view.context() else {
            return;
        };
        // The web context is shared by the app's webviews: keep the downloads of this one.
        let handler = context.connect_download_started({
            let view = view.clone();
            move |_, download| {
                if download.web_view().as_ref() != Some(&view) {
                    return;
                }
                track(&parent, download);
                let blank = view.uri().is_none_or(|uri| uri.is_empty());
                if let Some(popup) = popup.clone().filter(|_| blank) {
                    // Not inside the signal: the download keeps going without its webview.
                    glib::idle_add_local_once(move || {
                        let _ = popup.close();
                    });
                }
            }
        });
        let handler = Cell::new(Some(handler));
        view.connect_destroy(move |_| {
            if let Some(handler) = handler.take() {
                glib::signal::signal_handler_disconnect(&context, handler);
            }
        });
    }

    /// Destinations are `file://` URIs in the WebKitGTK 4.1 API.
    fn to_path(destination: &str) -> PathBuf {
        glib::filename_from_uri(destination)
            .map(|(path, _)| path)
            .unwrap_or_else(|_| PathBuf::from(destination))
    }

    fn track(parent: &Window, download: &Download) {
        let id = Rc::new(Cell::new(None));
        let total = |download: &Download| {
            download
                .response()
                .map(|r| r.content_length())
                .filter(|&n| n > 0)
        };
        download.connect_created_destination({
            let (parent, id) = (parent.clone(), id.clone());
            move |download, destination| {
                let url = download
                    .request()
                    .and_then(|r| r.uri())
                    .map(|uri| uri.to_string())
                    .unwrap_or_default();
                let cancel = {
                    let download = download.clone();
                    Box::new(move || download.cancel())
                };
                let path = to_path(destination);
                // WebKit picks the destination itself (`decide-destination` is not emitted for
                // the handler connected here) and reserves it with an empty file, renamed over
                // from `<path>.wkdownload` at the end: drop it, or the dialog would offer to
                // replace it.
                if fs::metadata(&path).is_ok_and(|m| m.len() == 0) {
                    let _ = fs::remove_file(&path);
                }
                id.set(Some(super::start(
                    &parent,
                    url,
                    path.clone(),
                    path,
                    total(download),
                    cancel,
                )));
            }
        });
        download.connect_received_data({
            let id = id.clone();
            move |download, _| {
                if let Some(id) = id.get() {
                    super::progress(id, download.received_data_length(), total(download));
                }
            }
        });
        let failed = Rc::new(Cell::new(false));
        download.connect_failed({
            let failed = failed.clone();
            move |_, _| failed.set(true)
        });
        download.connect_finished(move |download| {
            if let Some(id) = id.get() {
                let path = download.destination().map(|d| to_path(&d));
                super::finish(id, path, !failed.get());
            }
        });
    }
}

#[cfg(windows)]
mod webview2 {
    use std::path::PathBuf;

    use tauri::Window;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller, ICoreWebView2DownloadStartingEventArgs, ICoreWebView2_4,
        COREWEBVIEW2_DOWNLOAD_STATE, COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED,
        COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS,
    };
    use webview2_com::{
        take_pwstr, BytesReceivedChangedEventHandler, DownloadStartingEventHandler,
        StateChangedEventHandler,
    };
    use windows::core::{Interface, Result, HSTRING, PWSTR};

    pub fn setup(controller: ICoreWebView2Controller, parent: Window) {
        let result = unsafe {
            controller
                .CoreWebView2()
                .and_then(|core| core.cast::<ICoreWebView2_4>())
                .and_then(|core| {
                    let handler =
                        DownloadStartingEventHandler::create(Box::new(move |_, args| match args {
                            Some(args) => track(&parent, &args),
                            None => Ok(()),
                        }));
                    core.add_DownloadStarting(&handler, &mut 0)
                })
        };
        if let Err(e) = result {
            eprintln!("weboverlay: cannot track downloads: {e}");
        }
    }

    fn size(bytes: i64) -> Option<u64> {
        u64::try_from(bytes).ok().filter(|&n| n > 0)
    }

    fn track(parent: &Window, args: &ICoreWebView2DownloadStartingEventArgs) -> Result<()> {
        unsafe {
            let op = args.DownloadOperation()?;
            let mut uri = PWSTR::null();
            op.Uri(&mut uri)?;
            let mut path = PWSTR::null();
            args.ResultFilePath(&mut path)?;
            let suggested = PathBuf::from(take_pwstr(path));
            // WebView2 writes the final file directly: the dialog would offer to replace it.
            let mut part = suggested.clone().into_os_string();
            part.push(".part");
            args.SetResultFilePath(&HSTRING::from(part.as_os_str()))?;
            let mut total = 0;
            op.TotalBytesToReceive(&mut total)?;
            // No WebView2 download flyout: the downloads window replaces it.
            args.SetHandled(true)?;
            let cancel = {
                let op = op.clone();
                Box::new(move || {
                    let _ = op.Cancel();
                })
            };
            let id = super::start(
                parent,
                take_pwstr(uri),
                PathBuf::from(part),
                suggested,
                size(total),
                cancel,
            );
            op.add_BytesReceivedChanged(
                &BytesReceivedChangedEventHandler::create(Box::new(move |op, _| {
                    if let Some(op) = op {
                        let (mut received, mut total) = (0, 0);
                        op.BytesReceived(&mut received)?;
                        op.TotalBytesToReceive(&mut total)?;
                        super::progress(id, size(received).unwrap_or(0), size(total));
                    }
                    Ok(())
                })),
                &mut 0,
            )?;
            op.add_StateChanged(
                &StateChangedEventHandler::create(Box::new(move |op, _| {
                    let Some(op) = op else {
                        return Ok(());
                    };
                    let mut state = COREWEBVIEW2_DOWNLOAD_STATE::default();
                    op.State(&mut state)?;
                    if state != COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
                        let success = state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED;
                        let mut path = PWSTR::null();
                        op.ResultFilePath(&mut path)?;
                        let path = PathBuf::from(take_pwstr(path));
                        super::finish(id, Some(path), success);
                    }
                    Ok(())
                })),
                &mut 0,
            )?;
        }
        Ok(())
    }
}

/// URLs (redirect chains) found to be downloads by `probe`, with when: navigations to them stay
/// in the app for a while.
static EXPECTED: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());
const EXPECT_FOR: Duration = Duration::from_secs(60);

fn expected() -> MutexGuard<'static, Vec<(String, Instant)>> {
    let mut list = EXPECTED.lock().unwrap_or_else(|e| e.into_inner());
    list.retain(|(_, at)| at.elapsed() < EXPECT_FOR);
    list
}

/// `url` was probed as a download: the webview may load it.
pub fn is_expected(url: &Url) -> bool {
    expected().iter().any(|(u, _)| u == url.as_str())
}

/// Whether `url` (outside the allowed hosts) is a download, following its redirects without
/// cookies (`GET`, body not read: presigned URLs are method-specific). Blocking. A download
/// marks its redirect chain as expected, unless it already was: the webview then came back with
/// a URL spelled differently, and loading it again would loop.
pub fn probe(url: &Url, user_agent: Option<&str>) -> bool {
    let agent: ureq::Agent = icon::http_config()
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let mut chain = vec![url.clone()];
    for _ in 0..10 {
        let current = chain.last().expect("never empty");
        let Ok(response) = agent
            .get(current.as_str())
            .header("User-Agent", user_agent.unwrap_or(icon::USER_AGENT))
            .call()
        else {
            return false;
        };
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        if response.status().is_redirection() {
            let Some(next) = header("location").and_then(|l| current.join(&l).ok()) else {
                return false;
            };
            chain.push(next);
            continue;
        }
        if !response.status().is_success()
            || !is_download(
                header("content-disposition").as_deref(),
                header("content-type").as_deref(),
            )
        {
            return false;
        }
        let mut list = expected();
        if chain
            .iter()
            .any(|u| list.iter().any(|(e, _)| e == u.as_str()))
        {
            return false;
        }
        let now = Instant::now();
        list.extend(chain.iter().map(|u| (u.to_string(), now)));
        return true;
    }
    false
}

/// A response browsers save rather than show: `attachment`, or a type they don't render.
fn is_download(disposition: Option<&str>, content_type: Option<&str>) -> bool {
    if disposition.is_some_and(|d| {
        d.trim_start()
            .to_ascii_lowercase()
            .starts_with("attachment")
    }) {
        return true;
    }
    let Some(mime) = content_type
        .and_then(|t| t.split(';').next())
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
    else {
        return false;
    };
    let shown = ["text/", "image/", "audio/", "video/"]
        .iter()
        .any(|p| mime.starts_with(p))
        || mime.ends_with("+xml")
        || mime.ends_with("+json")
        || matches!(
            mime.as_str(),
            "application/xhtml+xml"
                | "application/xml"
                | "application/json"
                | "application/javascript"
                | "application/pdf"
        );
    !shown
}

/// Opens the downloads window, or focuses it when already open. Not from a sync command (see
/// `editor::open`): `open_later` there.
pub fn open(handle: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = handle.get_webview_window(DOWNLOADS_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        return window.set_focus();
    }
    let window = WebviewWindowBuilder::new(
        handle,
        DOWNLOADS_LABEL,
        WebviewUrl::App("downloads.html".into()),
    )
    .title("Downloads — WebOverlay")
    .inner_size(480.0, 420.0)
    .min_inner_size(320.0, 200.0)
    .visible(false)
    .on_page_load(show_when_loaded)
    .build()?;
    let _ = window.set_background_color(Some(theme_color(&window.as_ref().window(), PAGE_BG)));
    icon::set_default_taskbar_icon(&window.as_ref().window());
    Ok(())
}

/// `open` from a worker thread (see `editor::open_later`).
pub fn open_later(handle: &AppHandle) {
    let handle = handle.clone();
    std::thread::spawn(move || {
        if let Err(e) = open(&handle) {
            eprintln!("weboverlay: cannot open the downloads window: {e}");
        }
    });
}

fn check_caller(webview: &Webview) -> Result<(), String> {
    if webview.label() == DOWNLOADS_LABEL {
        Ok(())
    } else {
        Err("not allowed".into())
    }
}

/// Downloads of this process, newest first.
#[tauri::command]
pub fn downloads_list(webview: Webview) -> Result<Vec<View>, String> {
    check_caller(&webview)?;
    Ok(items()
        .iter()
        .rev()
        .map(|item| {
            let asking = item.target.is_none();
            let name = match &item.target {
                Some(Some(target)) => target.file_name(),
                _ => item.suggested.file_name(),
            };
            View {
                id: item.id,
                name: name
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                url: item.url.clone(),
                path: item.path.clone(),
                received: item.received,
                total: item.total,
                state: item.state,
                asking,
            }
        })
        .collect())
}

/// `cancel` / `open` / `reveal` / `remove` download `id`, or `clear` the finished ones.
#[tauri::command]
pub fn downloads_action(webview: Webview, action: String, id: Option<u64>) -> Result<(), String> {
    check_caller(&webview)?;
    // Sync command: runs on the main thread, where `CANCEL` lives.
    let finished = |item: &Item| item.state != State::Running && item.target.is_some();
    match action.as_str() {
        "clear" => {
            items().retain(|item| !finished(item));
            return Ok(());
        }
        "cancel" => {
            cancel(id.ok_or("missing id")?);
            return Ok(());
        }
        _ => {}
    }
    let id = id.ok_or("missing id")?;
    let mut list = items();
    let index = list
        .iter()
        .position(|i| i.id == id)
        .ok_or("unknown download")?;
    let item = &list[index];
    let done = item.state == State::Done && item.settled;
    let path = item.path.clone();
    match action.as_str() {
        "remove" if finished(item) => {
            list.remove(index);
            Ok(())
        }
        "open" if done => {
            drop(list);
            tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| e.to_string())
        }
        "reveal" if done => {
            drop(list);
            tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| e.to_string())
        }
        "remove" | "open" | "reveal" => Err(format!("cannot {action} now")),
        _ => Err(format!("unknown action {action:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("weboverlay-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn move_file_replaces_target() {
        let dir = temp_dir("dl");
        let (from, to) = (dir.join("a.txt"), dir.join("b.txt"));
        fs::write(&from, "new").unwrap();
        fs::write(&to, "old").unwrap();
        move_file(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "new");
        assert!(same_file(&to, &dir.join(".").join("b.txt")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn download_responses() {
        assert!(is_download(
            Some("attachment; filename=a.txt"),
            Some("text/plain")
        ));
        assert!(is_download(Some(" Attachment"), None));
        assert!(is_download(None, Some("application/octet-stream")));
        assert!(is_download(None, Some("application/zip; charset=binary")));
        assert!(is_download(None, Some("application/x-msdownload")));
        assert!(!is_download(
            Some("inline"),
            Some("text/html; charset=utf-8")
        ));
        assert!(!is_download(None, Some("image/png")));
        assert!(!is_download(None, Some("application/pdf")));
        assert!(!is_download(None, Some("image/svg+xml")));
        assert!(!is_download(None, Some("application/ld+json")));
        assert!(!is_download(None, None));
    }
}
