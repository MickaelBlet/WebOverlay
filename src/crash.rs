//! Web process failures of the header, content and popup webviews.
//!
//! - Crash (out of memory, ...): the webview stays blank, so the page is reloaded. A page that
//!   crashes again within `LOOP_WINDOW` is left alone (no reload loop; the local header is always
//!   reloaded): for the content, the header then shows the crash with a reload button.
//! - Content not responding (frozen page): the header shows it until the page responds again; its
//!   reload button then kills the web process (`kill`) and reloads the page.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{Manager, Webview};

use crate::header;
use crate::window;

const LOOP_WINDOW: Duration = Duration::from_secs(60);

/// Webview label -> time of its last crash.
static LAST: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());

/// Webviews whose web process `kill` terminated: their crash is a reload request.
#[cfg(windows)]
static KILLED: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Memory,
    Crash,
    Unresponsive,
}

impl Failure {
    pub fn message(self) -> &'static str {
        match self {
            Failure::Memory => "Page closed: out of memory",
            Failure::Crash => "Page crashed",
            Failure::Unresponsive => "Page not responding",
        }
    }
}

/// Watches the web process of `webview`.
pub fn setup(webview: &Webview) {
    #[cfg(target_os = "linux")]
    {
        let view = webview.clone();
        let _ = webview.with_webview(move |webview| linux::setup(webview.inner(), view));
    }
    #[cfg(windows)]
    {
        let view = webview.clone();
        let _ = webview.with_webview(move |webview| webview2::setup(webview.controller(), view));
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = webview;
}

/// Kills the (frozen) web process of `webview` and reloads its page.
pub fn kill(webview: &Webview) {
    #[cfg(target_os = "linux")]
    let _ = webview.with_webview(|webview| linux::kill(&webview.inner()));
    #[cfg(windows)]
    {
        let label = webview.label().to_owned();
        let view = webview.clone();
        let _ = webview.with_webview(move |webview| {
            lock(&KILLED).push(label.clone());
            if let Err(e) = webview2::kill(&webview.controller()) {
                eprintln!("weboverlay: cannot stop the web process: {e}");
                lock(&KILLED).retain(|l| *l != label);
                // Not inside `with_webview`, which holds the webview's lock.
                std::thread::spawn(move || view.reload());
            }
        });
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = webview.reload();
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a crash at `now` repeats one of `last` (and records it).
fn repeated(last: &mut Vec<(String, Instant)>, label: &str, now: Instant) -> bool {
    match last.iter_mut().find(|(l, _)| l == label) {
        Some((_, time)) => {
            let repeated = now.duration_since(*time) < LOOP_WINDOW;
            *time = now;
            repeated
        }
        None => {
            last.push((label.to_owned(), now));
            false
        }
    }
}

fn is_content(webview: &Webview) -> bool {
    webview.label() == window::content_label(webview.window().label())
}

/// Main thread, outside the crash event.
fn crashed(webview: &Webview, failure: Failure) {
    let label = webview.label();
    #[cfg(windows)]
    {
        let mut killed = lock(&KILLED);
        if let Some(i) = killed.iter().position(|l| l == label) {
            killed.remove(i);
            drop(killed);
            let _ = webview.reload();
            return;
        }
    }
    eprintln!("weboverlay: {label}: {}", failure.message());
    let window = webview.window();
    // The header is a local page: always reloaded.
    let repeated = label != window::header_label(window.label())
        && repeated(&mut lock(&LAST), label, Instant::now());
    if !repeated {
        let _ = webview.reload();
    } else if is_content(webview) {
        header::set_failure(webview.app_handle(), window.label(), Some(failure));
    }
}

/// The web process of `webview` stopped (`false`) or started again (`true`) responding.
fn responsive(webview: &Webview, ok: bool) {
    if !is_content(webview) {
        return;
    }
    let handle = webview.app_handle();
    let label = webview.window().label().to_owned();
    if ok {
        if header::failure(handle, &label) == Some(Failure::Unresponsive) {
            header::set_failure(handle, &label, None);
        }
        return;
    }
    eprintln!("weboverlay: {}: page not responding", webview.label());
    if header::failure(handle, &label).is_none() {
        header::set_failure(handle, &label, Some(Failure::Unresponsive));
    }
    // WebView2 has no "responsive again" event: a script runs once the page responds.
    #[cfg(windows)]
    {
        let view = webview.clone();
        let _ = webview.eval_with_callback("0", move |_| responsive(&view, true));
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use tauri::Webview;
    use webkit2gtk::glib;
    use webkit2gtk::{WebProcessTerminationReason, WebView, WebViewExt};

    use super::Failure;

    pub fn setup(view: WebView, webview: Webview) {
        view.connect_web_process_terminated({
            let webview = webview.clone();
            move |_, reason| {
                let failure = match reason {
                    // `kill`: a reload request.
                    WebProcessTerminationReason::TerminatedByApi => None,
                    WebProcessTerminationReason::ExceededMemoryLimit => Some(Failure::Memory),
                    _ => Some(Failure::Crash),
                };
                let webview = webview.clone();
                // Outside the signal (a reload in it is ignored).
                glib::MainContext::default().spawn_local(async move {
                    match failure {
                        Some(failure) => super::crashed(&webview, failure),
                        None => {
                            let _ = webview.reload();
                        }
                    }
                });
            }
        });
        view.connect_is_web_process_responsive_notify(move |view| {
            let responsive = view.is_web_process_responsive();
            let webview = webview.clone();
            glib::MainContext::default()
                .spawn_local(async move { super::responsive(&webview, responsive) });
        });
    }

    /// Reloaded by the `web-process-terminated` handler (a reload now would be ignored).
    pub fn kill(view: &WebView) {
        // Not inside `with_webview`: Tauri runs it holding the webview's lock, and the handlers
        // WebKit calls synchronously (title, ...) take it again.
        let view = view.clone();
        glib::MainContext::default().spawn_local(async move { view.terminate_web_process() });
    }
}

#[cfg(windows)]
mod webview2 {
    use tauri::Webview;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller, ICoreWebView2Environment13, ICoreWebView2FrameInfo2,
        ICoreWebView2ProcessExtendedInfoCollection, ICoreWebView2ProcessFailedEventArgs,
        ICoreWebView2ProcessFailedEventArgs2, ICoreWebView2_2, ICoreWebView2_20,
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        COREWEBVIEW2_PROCESS_FAILED_REASON, COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY,
    };
    use webview2_com::{GetProcessExtendedInfosCompletedHandler, ProcessFailedEventHandler};
    use windows::core::{Interface, Result, BOOL};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    use super::Failure;

    pub fn setup(controller: ICoreWebView2Controller, webview: Webview) {
        let result = unsafe {
            controller.CoreWebView2().and_then(|core| {
                let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    if let Some(failure) = args.map(|args| failure(&args)).transpose()?.flatten() {
                        // Not inside the event: `reload` may run inline on the main thread.
                        let webview = webview.clone();
                        let _ = webview.clone().run_on_main_thread(move || match failure {
                            Failure::Unresponsive => super::responsive(&webview, false),
                            _ => super::crashed(&webview, failure),
                        });
                    }
                    Ok(())
                }));
                core.add_ProcessFailed(&handler, &mut 0)
            })
        };
        if let Err(e) = result {
            eprintln!("weboverlay: cannot watch the web process: {e}");
        }
    }

    /// `None` for the failures WebView2 recovers from (GPU, utility processes, iframes).
    fn failure(args: &ICoreWebView2ProcessFailedEventArgs) -> Result<Option<Failure>> {
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        unsafe { args.ProcessFailedKind(&mut kind)? };
        if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE {
            return Ok(Some(Failure::Unresponsive));
        }
        if kind != COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
            return Ok(None);
        }
        let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
        let memory = args
            .cast::<ICoreWebView2ProcessFailedEventArgs2>()
            .and_then(|args| unsafe { args.Reason(&mut reason) })
            .is_ok_and(|()| reason == COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY);
        Ok(Some(if memory {
            Failure::Memory
        } else {
            Failure::Crash
        }))
    }

    /// Terminates the renderer process of the main frame: WebView2 then reports it as exited.
    pub fn kill(controller: &ICoreWebView2Controller) -> Result<()> {
        unsafe {
            let core = controller.CoreWebView2()?;
            let mut frame = 0;
            core.cast::<ICoreWebView2_20>()?.FrameId(&mut frame)?;
            let environment = core
                .cast::<ICoreWebView2_2>()?
                .Environment()?
                .cast::<ICoreWebView2Environment13>()?;
            let handler =
                GetProcessExtendedInfosCompletedHandler::create(Box::new(move |result, infos| {
                    result?;
                    match infos
                        .map(|infos| renderer(&infos, frame))
                        .transpose()?
                        .flatten()
                    {
                        Some(pid) => {
                            let process = OpenProcess(PROCESS_TERMINATE, false, pid)?;
                            let result = TerminateProcess(process, 1);
                            let _ = CloseHandle(process);
                            result
                        }
                        None => Err(windows::core::Error::empty()),
                    }
                    .inspect_err(|e| eprintln!("weboverlay: cannot stop the web process: {e}"))
                }));
            environment.GetProcessExtendedInfos(&handler)
        }
    }

    /// Process id of the renderer running frame `frame`.
    fn renderer(
        infos: &ICoreWebView2ProcessExtendedInfoCollection,
        frame: u32,
    ) -> Result<Option<u32>> {
        unsafe {
            let mut count = 0;
            infos.Count(&mut count)?;
            for i in 0..count {
                let info = infos.GetValueAtIndex(i)?;
                let frames = info.AssociatedFrameInfos()?.GetIterator()?;
                let mut has = BOOL(0);
                frames.HasCurrent(&mut has)?;
                while has.as_bool() {
                    let mut id = 0;
                    if frames
                        .GetCurrent()?
                        .cast::<ICoreWebView2FrameInfo2>()
                        .and_then(|f| f.FrameId(&mut id))
                        .is_ok_and(|()| id == frame)
                    {
                        let mut pid = 0;
                        info.ProcessInfo()?.ProcessId(&mut pid)?;
                        return Ok(u32::try_from(pid).ok());
                    }
                    frames.MoveNext(&mut has)?;
                }
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_loop_detection() {
        let mut last = Vec::new();
        let t0 = Instant::now();
        assert!(!repeated(&mut last, "a:content", t0));
        assert!(!repeated(&mut last, "b:content", t0));
        assert!(repeated(
            &mut last,
            "a:content",
            t0 + Duration::from_secs(10)
        ));
        assert!(!repeated(
            &mut last,
            "a:content",
            t0 + Duration::from_secs(10) + LOOP_WINDOW
        ));
    }
}
