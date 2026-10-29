//! Config editor window (`dist/editor.html`): edits `config.toml` and the script/style files
//! it references. Only the local editor webview may call its commands, and only on those files.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, Manager, State, Webview, WebviewUrl, WebviewWindowBuilder};

use crate::config::{self, Config};
use crate::icon;
use crate::window::{show_when_loaded, theme_color, PAGE_BG};

/// Contains `:`, so it can't clash with an app window label (`[A-Za-z0-9_-]+`).
pub const EDITOR_LABEL: &str = "weboverlay:editor";

pub struct Editor {
    pub config_path: PathBuf,
    /// Command line for Restart: the current one without `--edit`.
    pub relaunch_args: Vec<OsString>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Config,
    Script,
    Style,
}

#[derive(Debug, Serialize)]
struct FileEntry {
    path: PathBuf,
    kind: Kind,
    /// App using the file (first one when shared), empty for the config.
    app: String,
    exists: bool,
}

#[derive(Serialize)]
pub struct Files {
    files: Vec<FileEntry>,
    /// Why the saved config can't be parsed (only the config is listed then).
    error: Option<String>,
}

#[derive(Serialize)]
pub struct FileContent {
    content: String,
    exists: bool,
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
            .visible(false)
            .on_page_load(show_when_loaded)
            .build()?;
    let _ = window.set_background_color(Some(theme_color(&window.as_ref().window(), PAGE_BG)));
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

/// `args` (without the program name) minus `-e` / `--edit`.
pub fn strip_edit_flag(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    args.into_iter()
        .filter(|arg| arg != "-e" && arg != "--edit")
        .collect()
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
        Err(_) => return Files { files, error: None },
    };
    let config = match Config::parse_at(&text, config_path) {
        Ok(config) => config,
        Err(e) => {
            return Files {
                files,
                error: Some(format!("{e:#}")),
            }
        }
    };
    let mut seen: HashSet<PathBuf> = HashSet::from([config_path.to_owned()]);
    for app in &config.apps {
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
    Files { files, error: None }
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

/// Relaunches the app (without `--edit`) and exits this instance.
#[tauri::command]
pub fn editor_restart(
    webview: Webview,
    editor: State<'_, Editor>,
    handle: AppHandle,
) -> Result<(), String> {
    check_caller(&webview)?;
    // The new instance would exit silently on a config error: report it here instead.
    Config::load(&editor.config_path).map_err(|e| format!("{e:#}"))?;
    let exe = config::exe_path().ok_or("cannot determine the executable path")?;
    Command::new(&exe)
        .args(&editor.relaunch_args)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
    handle.exit(0);
    Ok(())
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
    fn strips_edit_flag() {
        let args = ["--edit", "-c", "a.toml", "-e", "demo"].map(OsString::from);
        assert_eq!(strip_edit_flag(args), ["-c", "a.toml", "demo"]);
    }

    #[test]
    fn lists_config_files() {
        let dir = temp_dir("files");
        let config = dir.join("config.toml");
        fs::write(
            &config,
            r#"
            [[app]]
            name = "a"
            url = "https://a.com"
            scripts = [{ path = "a/main.js" }]
            styles = [{ path = "shared.css" }]
            [[app]]
            name = "b"
            url = "https://b.com"
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

        fs::write(&config, "[[app]]\nname = \"a b\"").unwrap();
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
}
