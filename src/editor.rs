//! Config editor window (`dist/editor.html`): edits `config.toml` and the script/style files
//! it references. Only the local editor webview may call its commands, and only on those files.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, Webview, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

use crate::config::Config;
use crate::schema::{KeyDoc, SCHEMA};
use crate::window::{show_when_loaded, theme_color, PAGE_BG};
use crate::{icon, launcher, window};

/// Contains `:`, so it can't clash with an app window label (`[A-Za-z0-9_-]+`).
pub const EDITOR_LABEL: &str = "weboverlay:editor";

pub struct Editor {
    pub config_path: PathBuf,
    /// `--maximized`, applied by Restart.
    pub maximized: bool,
    /// The page has unsaved changes (`editor_dirty`): closing the window asks first.
    pub dirty: AtomicBool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Config,
    Script,
    Style,
}

#[derive(Debug, Serialize)]
struct FileEntry {
    path: PathBuf,
    kind: Kind,
    /// App using the file, empty for the config.
    app: String,
    exists: bool,
}

#[derive(Serialize)]
pub struct Files {
    files: Vec<FileEntry>,
    /// Name and title of the app of the saved config.
    app: Option<AppInfo>,
    /// Why the saved config can't be parsed (only the config is listed then).
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct AppInfo {
    name: String,
    title: Option<String>,
    /// `data:` URL: the configured icon, else the last favicon seen by the app.
    icon: Option<String>,
}

#[derive(Serialize)]
pub struct FileContent {
    content: String,
    exists: bool,
}

/// Why a config text is invalid, with its location when known.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Diagnostic {
    message: String,
    /// UTF-16 offsets (JS string indexes) of the faulty text.
    start: Option<usize>,
    end: Option<usize>,
}

/// Opens the editor window, or focuses it when already open.
/// From a sync command use `open_later`: building a window there leaves it blank on Windows.
pub fn open(handle: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = handle.get_webview_window(EDITOR_LABEL) {
        let _ = window.unminimize();
        return window.set_focus();
    }
    let config_path = &handle.state::<Editor>().config_path;
    let window =
        WebviewWindowBuilder::new(handle, EDITOR_LABEL, WebviewUrl::App("editor.html".into()))
            .title(format!("WebOverlay — {}", config_path.display()))
            .inner_size(1000.0, 700.0)
            .min_inner_size(520.0, 320.0)
            .decorations(false)
            // Else WebView2 swallows the HTML5 drag and drop of the tabs (Windows).
            .disable_drag_drop_handler()
            .visible(false)
            .on_page_load(show_when_loaded)
            .build()?;
    let _ = window.set_background_color(Some(theme_color(&window.as_ref().window(), PAGE_BG)));
    let handle = handle.clone();
    let webview = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            if handle.state::<Editor>().dirty.load(Ordering::Relaxed) {
                api.prevent_close();
                let _ = webview.eval("window.editorCloseRequested?.()");
            }
        }
    });
    icon::set_default_taskbar_icon(&window.as_ref().window());
    Ok(())
}

/// `open` from a worker thread: Tauri then builds the window from the event loop, after the
/// calling command returned. (`run_on_main_thread` runs inline when already on the main thread.)
pub fn open_later(handle: &AppHandle) {
    let handle = handle.clone();
    std::thread::spawn(move || {
        if let Err(e) = open(&handle) {
            eprintln!("weboverlay: cannot open the editor: {e}");
        }
    });
}

/// The config plus the scripts/styles of the config saved at `config_path`.
fn files(config_path: &Path) -> Files {
    let mut files = vec![FileEntry {
        path: config_path.to_owned(),
        kind: Kind::Config,
        app: String::new(),
        exists: config_path.is_file(),
    }];
    let text = match fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(_) => {
            return Files {
                files,
                app: None,
                error: None,
            }
        }
    };
    let config = match Config::parse_at(&text, config_path) {
        Ok(config) => config,
        Err(e) => {
            return Files {
                files,
                app: None,
                error: Some(format!("{e:#}")),
            }
        }
    };
    let mut seen: HashSet<PathBuf> = HashSet::from([config_path.to_owned()]);
    if let Some(app) = &config.app {
        let scripts = app.scripts.iter().map(|s| (&s.path, Kind::Script));
        let styles = app.styles.iter().map(|s| (&s.path, Kind::Style));
        for (path, kind) in scripts.chain(styles) {
            if seen.insert(path.clone()) {
                files.push(FileEntry {
                    path: path.clone(),
                    kind,
                    app: app.name.clone(),
                    exists: path.is_file(),
                });
            }
        }
    }
    let app = config.app.map(|app| AppInfo {
        icon: app
            .icon
            .as_deref()
            .and_then(icon::file_data_url)
            .or_else(|| icon::cached(config_path, &app.name)),
        name: app.name,
        title: app.title,
    });
    Files {
        files,
        app,
        error: None,
    }
}

fn check_caller(webview: &Webview) -> Result<(), String> {
    if webview.label() == EDITOR_LABEL {
        Ok(())
    } else {
        Err("not allowed".into())
    }
}

/// Fails unless `path` is one of the editable files.
fn editable(editor: &Editor, path: &Path) -> Result<Kind, String> {
    files(&editor.config_path)
        .files
        .into_iter()
        .find(|file| file.path == path)
        .map(|file| file.kind)
        .ok_or_else(|| format!("{} is not an editable file", path.display()))
}

fn read_file(path: &Path) -> Result<FileContent, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(FileContent {
            content: text.replace("\r\n", "\n"),
            exists: true,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileContent {
            content: String::new(),
            exists: false,
        }),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// Writes `content`, keeping CRLF line endings when the existing file uses them.
fn write_file(path: &Path, content: &str) -> Result<(), String> {
    let crlf = fs::read_to_string(path).is_ok_and(|old| old.contains("\r\n"));
    let content = if crlf {
        content.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        content.to_owned()
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(path, content).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

// ---- Adding, renaming and removing script/style entries of the saved config ----

impl Kind {
    fn key(self) -> Result<&'static str, String> {
        match self {
            Kind::Script => Ok("scripts"),
            Kind::Style => Ok("styles"),
            Kind::Config => Err("the config is not a script or a style".into()),
        }
    }
}

/// Checks `rel` (as typed in the page) and returns it with `/` separators: a relative path
/// under the config directory with the extension of `kind`.
fn entry_path(kind: Kind, rel: &str) -> Result<String, String> {
    let rel = rel.trim().replace('\\', "/");
    let path = Path::new(&rel);
    if rel.is_empty() || path.is_absolute() || rel.starts_with('/') {
        return Err("give a path relative to the config directory".into());
    }
    if path
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("the path must stay in the config directory (no `..`)".into());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let ok = match kind {
        Kind::Script => matches!(ext.as_str(), "js" | "mjs" | "cjs"),
        Kind::Style => ext == "css",
        Kind::Config => false,
    };
    if !ok {
        let want = if kind == Kind::Style { ".css" } else { ".js" };
        return Err(format!("a {} file name ends with {want}", kind.key()?));
    }
    Ok(rel)
}

/// The `[app]` table of `doc`.
fn app_table(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    doc.get_mut("app")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| "the config has no [app] table: add one first".into())
}

/// Appends an entry to `app.<key>`, as an inline table (`key = [...]`) or a `[[app.<key>]]`
/// table depending on the form already used.
fn push_entry(doc: &mut DocumentMut, key: &str, entry: InlineTable) -> Result<(), String> {
    let app = app_table(doc)?;
    match app.get_mut(key) {
        None => {
            let mut array = Array::new();
            array.push(entry);
            app.insert(key, toml_edit::value(array));
        }
        Some(Item::ArrayOfTables(tables)) => tables.push(entry.into_table()),
        Some(Item::Value(Value::Array(array))) => {
            // A multi-line array keeps one entry per line.
            let multiline = array.iter().any(|v| {
                v.decor()
                    .prefix()
                    .is_some_and(|p| p.as_str().is_some_and(|p| p.contains('\n')))
            });
            array.push(entry);
            if multiline {
                let n = array.len();
                let indent = array
                    .get(0)
                    .and_then(|v| v.decor().prefix())
                    .and_then(|p| p.as_str())
                    .unwrap_or("\n  ")
                    .to_owned();
                if let Some(last) = array.get_mut(n - 1) {
                    last.decor_mut().set_prefix(indent);
                }
                array.set_trailing_comma(true);
                if !array.trailing().as_str().is_some_and(|t| t.contains('\n')) {
                    array.set_trailing("\n");
                }
            }
        }
        Some(_) => return Err(format!("app.{key} is not a list")),
    }
    Ok(())
}

/// Calls `f` with the `path` value (relative, as written) of every entry of `app.scripts` and
/// `app.styles`; `f` returns whether to keep the entry.
fn retain_entries(
    doc: &mut DocumentMut,
    mut f: impl FnMut(&mut Value) -> bool,
) -> Result<(), String> {
    let app = app_table(doc)?;
    for key in ["scripts", "styles"] {
        match app.get_mut(key) {
            Some(Item::ArrayOfTables(tables)) => {
                let mut keep = Vec::new();
                for t in tables.iter_mut() {
                    keep.push(match t.get_mut("path").and_then(Item::as_value_mut) {
                        Some(v) => f(v),
                        None => true,
                    });
                }
                let mut i = 0;
                tables.retain(|_| {
                    i += 1;
                    keep[i - 1]
                });
                if tables.is_empty() {
                    app.remove(key);
                }
            }
            Some(Item::Value(Value::Array(array))) => {
                let mut keep = Vec::new();
                for v in array.iter_mut() {
                    keep.push(
                        match v.as_inline_table_mut().and_then(|t| t.get_mut("path")) {
                            Some(v) => f(v),
                            None => true,
                        },
                    );
                }
                let mut i = 0;
                array.retain(|_| {
                    i += 1;
                    keep[i - 1]
                });
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether the entry path value `v` (relative to `base`) designates `path`.
fn designates(v: &Value, base: &Path, path: &Path) -> bool {
    v.as_str().is_some_and(|s| base.join(s) == path)
}

/// Applies `f` to the saved config document, validates the result and writes it.
fn edit_config(
    config_path: &Path,
    f: impl FnOnce(&mut DocumentMut, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let text = fs::read_to_string(config_path)
        .map_err(|e| format!("cannot read {}: {e}", config_path.display()))?;
    let mut doc: DocumentMut = text
        .replace("\r\n", "\n")
        .parse()
        .map_err(|e| format!("invalid config: {e}"))?;
    let base = config_path.parent().unwrap_or(Path::new("."));
    f(&mut doc, base)?;
    let out = doc.to_string();
    Config::parse(&out).map_err(|e| format!("invalid config: {e:#}"))?;
    write_file(config_path, &out)
}

/// The entry kind of `path` in the saved config (scripts and styles only).
fn entry_kind(editor: &Editor, path: &Path) -> Result<Kind, String> {
    match editable(editor, path)? {
        Kind::Config => Err("the config itself cannot be changed this way".into()),
        kind => Ok(kind),
    }
}

/// Adds a script or style entry to the saved config and creates its file when missing.
/// `path` is relative to the config directory. Returns the absolute path.
fn add(
    config_path: &Path,
    kind: Kind,
    path: &str,
    run_at: Option<&str>,
    matches: &[String],
) -> Result<PathBuf, String> {
    let rel = entry_path(kind, path)?;
    let base = config_path.parent().unwrap_or(Path::new("."));
    let full = base.join(&rel);
    let key = kind.key()?;
    edit_config(config_path, |doc, _| {
        let mut entry = InlineTable::new();
        entry.insert("path", rel.as_str().into());
        if let Some(run_at) = run_at.filter(|r| kind == Kind::Script && *r != "document-end") {
            entry.insert("run_at", run_at.into());
        }
        if !matches.is_empty() {
            entry.insert("matches", matches.iter().collect::<Array>().into());
        }
        push_entry(doc, key, entry)
    })?;
    if !full.exists() {
        write_file(&full, "")?;
    }
    Ok(full)
}

/// Removes the entries of `path` from the saved config (the file is kept).
fn unreference(config_path: &Path, path: &Path) -> Result<(), String> {
    edit_config(config_path, |doc, base| {
        retain_entries(doc, |v| !designates(v, base, path))
    })
}

/// Renames the file `path` to `new` (relative to the config directory) and its entries.
fn rename(config_path: &Path, kind: Kind, path: &Path, new: &str) -> Result<PathBuf, String> {
    let rel = entry_path(kind, new)?;
    let base = config_path.parent().unwrap_or(Path::new("."));
    let full = base.join(&rel);
    if full == path {
        return Ok(full);
    }
    if full.exists() {
        return Err(format!("{} already exists", full.display()));
    }
    edit_config(config_path, |doc, base| {
        retain_entries(doc, |v| {
            if designates(v, base, path) {
                let decor = v.decor().clone();
                *v = rel.as_str().into();
                *v.decor_mut() = decor;
            }
            true
        })
    })?;
    if path.exists() {
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::rename(path, &full).map_err(|e| format!("cannot rename {}: {e}", path.display()))?;
    }
    Ok(full)
}

#[tauri::command]
pub fn editor_add(
    webview: Webview,
    editor: State<'_, Editor>,
    kind: Kind,
    path: String,
    run_at: Option<String>,
    matches: Vec<String>,
) -> Result<PathBuf, String> {
    check_caller(&webview)?;
    add(
        &editor.config_path,
        kind,
        &path,
        run_at.as_deref(),
        &matches,
    )
}

#[tauri::command]
pub fn editor_unreference(
    webview: Webview,
    editor: State<'_, Editor>,
    path: PathBuf,
) -> Result<(), String> {
    check_caller(&webview)?;
    entry_kind(&editor, &path)?;
    unreference(&editor.config_path, &path)
}

#[tauri::command]
pub fn editor_rename(
    webview: Webview,
    editor: State<'_, Editor>,
    path: PathBuf,
    new_path: String,
) -> Result<PathBuf, String> {
    check_caller(&webview)?;
    let kind = entry_kind(&editor, &path)?;
    rename(&editor.config_path, kind, &path, &new_path)
}

/// Removes the entries of `path` and deletes the file.
#[tauri::command]
pub fn editor_delete(
    webview: Webview,
    editor: State<'_, Editor>,
    path: PathBuf,
) -> Result<(), String> {
    check_caller(&webview)?;
    entry_kind(&editor, &path)?;
    unreference(&editor.config_path, &path)?;
    match fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("cannot delete {}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Shows an editable file (or its directory when missing) in the file manager.
#[tauri::command]
pub fn editor_reveal(
    webview: Webview,
    editor: State<'_, Editor>,
    path: PathBuf,
) -> Result<(), String> {
    check_caller(&webview)?;
    editable(&editor, &path)?;
    let target = if path.exists() {
        path
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or(path)
    };
    tauri_plugin_opener::reveal_item_in_dir(&target).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn editor_files(webview: Webview, editor: State<'_, Editor>) -> Result<Files, String> {
    check_caller(&webview)?;
    Ok(files(&editor.config_path))
}

#[tauri::command]
pub fn editor_read(
    webview: Webview,
    editor: State<'_, Editor>,
    path: PathBuf,
) -> Result<FileContent, String> {
    check_caller(&webview)?;
    editable(&editor, &path)?;
    read_file(&path)
}

#[tauri::command]
pub fn editor_write(
    webview: Webview,
    editor: State<'_, Editor>,
    path: PathBuf,
    content: String,
) -> Result<(), String> {
    check_caller(&webview)?;
    if editable(&editor, &path)? == Kind::Config {
        // The config on disk stays valid: refuse to save a broken one.
        Config::parse(&content).map_err(|e| format!("invalid config: {e:#}"))?;
    }
    write_file(&path, &content)
}

#[tauri::command]
pub fn editor_schema(webview: Webview) -> Result<&'static [KeyDoc], String> {
    check_caller(&webview)?;
    Ok(SCHEMA)
}

/// Keeps the editor window above the other windows (`pinned`) or not; returns the new state.
#[tauri::command]
pub fn editor_pin(webview: Webview, pinned: bool) -> Result<bool, String> {
    check_caller(&webview)?;
    webview
        .window()
        .set_always_on_top(pinned)
        .map_err(|e| e.to_string())?;
    Ok(pinned)
}

/// Window controls of the undecorated editor (its page draws the title bar); `force_close`
/// closes it without asking. Returns whether the window is maximized.
#[tauri::command]
pub fn editor_window(webview: Webview, action: String) -> Result<bool, String> {
    check_caller(&webview)?;
    let window = webview.window();
    if action == "force_close" {
        window.destroy().map_err(|e| e.to_string())?;
        return Ok(false);
    }
    window::window_action(&window, &action)
}

/// Whether the page has unsaved changes (closing the window then asks the page first).
#[tauri::command]
pub fn editor_dirty(webview: Webview, editor: State<'_, Editor>, dirty: bool) {
    if check_caller(&webview).is_ok() {
        editor.dirty.store(dirty, Ordering::Relaxed);
    }
}

/// Validates an unsaved config text (live diagnostics); touches no file.
#[tauri::command]
pub fn editor_check(webview: Webview, content: String) -> Result<Option<Diagnostic>, String> {
    check_caller(&webview)?;
    Ok(check(&content))
}

fn check(content: &str) -> Option<Diagnostic> {
    let error = Config::parse(content).err()?;
    let utf16 = |byte: usize| content.get(..byte).map(|s| s.encode_utf16().count());
    match error.downcast_ref::<toml::de::Error>() {
        Some(toml_error) => {
            let span = toml_error.span();
            Some(Diagnostic {
                message: toml_error.message().to_owned(),
                start: span.as_ref().and_then(|s| utf16(s.start)),
                end: span.as_ref().and_then(|s| utf16(s.end)),
            })
        }
        None => Some(Diagnostic {
            message: format!("{error:#}"),
            start: None,
            end: None,
        }),
    }
}

/// Reopens the app window with the saved config, in this process: the editor stays open.
/// Popups are closed too; the downloads window stays.
#[tauri::command]
pub async fn editor_restart(
    webview: Webview,
    editor: State<'_, Editor>,
    handle: AppHandle,
) -> Result<(), String> {
    check_caller(&webview)?;
    // Everything is checked before closing anything.
    let app =
        crate::load_app(&editor.config_path, editor.maximized).map_err(|e| format!("{e:#}"))?;
    launcher::record(&editor.config_path, &app.config);
    let old: Vec<_> = handle
        .windows()
        .into_values()
        .filter(|window| !window.label().starts_with("weboverlay:"))
        .collect();
    for window in &old {
        let _ = window.destroy();
    }
    // The new windows reuse the labels: wait until the old ones are gone.
    let deadline = Instant::now() + Duration::from_secs(5);
    while old.iter().any(|w| handle.get_window(w.label()).is_some()) {
        if Instant::now() > deadline {
            return Err("the previous windows did not close".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    window::open(&handle, app).map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("weboverlay-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_config_files() {
        let dir = temp_dir("files");
        let config = dir.join("config.toml");
        fs::write(
            &config,
            r#"
            [app]
            name = "a"
            url = "https://a.com"
            scripts = [{ path = "a/main.js" }, { path = "a/main.js", run_at = "document-idle" }]
            styles = [{ path = "shared.css" }]
            "#,
        )
        .unwrap();
        fs::write(dir.join("shared.css"), "").unwrap();
        let list = files(&config);
        assert!(list.error.is_none());
        let paths: Vec<_> = list.files.iter().map(|f| f.path.clone()).collect();
        assert_eq!(
            paths,
            [
                config.clone(),
                dir.join("a/main.js"),
                dir.join("shared.css")
            ]
        );
        assert_eq!(list.files[1].kind, Kind::Script);
        assert!(!list.files[1].exists);
        assert_eq!(list.files[2].app, "a");
        assert!(list.files[2].exists);

        fs::write(&config, "[app]\nname = \"a b\"").unwrap();
        let list = files(&config);
        assert!(list.error.is_some());
        assert_eq!(list.files.len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_config_is_listed() {
        let dir = temp_dir("missing");
        let list = files(&dir.join("config.toml"));
        assert!(list.error.is_none());
        assert_eq!(list.files.len(), 1);
        assert!(!list.files[0].exists);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn checks_config_text() {
        let ok = "[app]\nname = \"a\"\nurl = \"https://a.com\"";
        assert_eq!(check(ok), None);
        // "é" is 2 bytes but 1 UTF-16 unit.
        let typo = "# é\n[app]\nname = \"a\"\nurl = \"https://a.com\"\nwidht = 1";
        let d = check(typo).unwrap();
        assert!(d.message.contains("widht"), "{}", d.message);
        let at = typo.find("widht").unwrap() - 1;
        assert_eq!((d.start, d.end), (Some(at), Some(at + 5)));
        let semantic = check("[app]\nname = \"a b\"\nurl = \"https://a.com\"").unwrap();
        assert!(semantic.message.contains("must match"));
        assert_eq!(semantic.start, None);
        let list = check("[[app]]\nname = \"a\"\nurl = \"https://a.com\"").unwrap();
        assert!(
            list.message.contains("one app per config"),
            "{}",
            list.message
        );
        assert!(list.start.is_some());
    }

    #[test]
    fn keeps_crlf_and_creates_dirs() {
        let dir = temp_dir("crlf");
        let path = dir.join("sub/a.js");
        write_file(&path, "a\nb\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\nb\n");
        fs::write(&path, "a\r\nb\r\n").unwrap();
        let read = read_file(&path).unwrap();
        assert_eq!(read.content, "a\nb\n");
        write_file(&path, "x\ny\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "x\r\ny\r\n");
        let missing = read_file(&dir.join("nope.js")).unwrap();
        assert!(!missing.exists);
        fs::remove_dir_all(dir).unwrap();
    }

    const APP: &str = "# my app\n[app]\nname = \"a\" # inline\nurl = \"https://a.com\"\n";

    #[test]
    fn adds_entries() {
        let dir = temp_dir("add");
        let config = dir.join("config.toml");
        fs::write(&config, APP).unwrap();
        let full = add(
            &config,
            Kind::Script,
            "js\\main.js",
            Some("document-idle"),
            &[],
        )
        .unwrap();
        assert_eq!(full, dir.join("js/main.js"));
        assert!(full.is_file());
        add(&config, Kind::Style, "a.css", None, &["*://a.com/*".into()]).unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(
            text.starts_with("# my app\n[app]\nname = \"a\" # inline\n"),
            "{text}"
        );
        assert!(
            text.contains(r#"scripts = [{ path = "js/main.js", run_at = "document-idle" }]"#),
            "{text}"
        );
        assert!(
            text.contains(r#"styles = [{ path = "a.css", matches = ["*://a.com/*"] }]"#),
            "{text}"
        );
        let list = files(&config);
        assert_eq!(list.files.len(), 3);
        assert_eq!(list.app.as_ref().unwrap().name, "a");

        // Multi-line arrays get one entry per line; tables of arrays get a table.
        fs::write(
            &config,
            format!("{APP}scripts = [\n  {{ path = \"x.js\" }},\n]\n\n[[app.styles]]\npath = \"x.css\"\n"),
        )
        .unwrap();
        add(&config, Kind::Script, "y.js", None, &[]).unwrap();
        add(&config, Kind::Style, "y.css", None, &[]).unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(
            text.contains("scripts = [\n  { path = \"x.js\" },\n  { path = \"y.js\" },\n]"),
            "{text}"
        );
        assert!(text.contains("[[app.styles]]\npath = \"y.css\""), "{text}");

        for bad in ["../x.js", "a/../../x.js", "/x.js", "x.css", ""] {
            assert!(add(&config, Kind::Script, bad, None, &[]).is_err(), "{bad}");
        }
        fs::write(&config, "").unwrap();
        assert!(add(&config, Kind::Script, "x.js", None, &[])
            .unwrap_err()
            .contains("[app]"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn renames_and_unreferences() {
        let dir = temp_dir("rename");
        let config = dir.join("config.toml");
        fs::write(
            &config,
            format!("{APP}scripts = [{{ path = \"a.js\" }}, {{ path = \"b.js\" }}]\n\n[[app.styles]]\npath = \"a.css\" # keep\n"),
        )
        .unwrap();
        fs::write(dir.join("a.js"), "1").unwrap();
        let new = rename(&config, Kind::Script, &dir.join("a.js"), "sub/c.js").unwrap();
        assert_eq!(fs::read_to_string(new).unwrap(), "1");
        assert!(!dir.join("a.js").exists());
        fs::write(dir.join("b.js"), "").unwrap();
        assert!(rename(&config, Kind::Script, &dir.join("sub/c.js"), "b.js").is_err());
        rename(&config, Kind::Style, &dir.join("a.css"), "d.css").unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(
            text.contains(r#"scripts = [{ path = "sub/c.js" }, { path = "b.js" }]"#),
            "{text}"
        );
        assert!(text.contains("path = \"d.css\" # keep"), "{text}");

        unreference(&config, &dir.join("b.js")).unwrap();
        unreference(&config, &dir.join("d.css")).unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(
            text.contains(r#"scripts = [{ path = "sub/c.js" }]"#),
            "{text}"
        );
        assert!(!text.contains("styles"), "{text}");
        assert!(dir.join("b.js").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
