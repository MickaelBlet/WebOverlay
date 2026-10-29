//! Launcher window (`dist/launcher.html`), shown when started without app nor `--config`:
//! recent apps plus the apps of the user, portable and added configs, each started in a new
//! process. Only the launcher webview may call its commands, and only on the listed configs/apps.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Webview, WebviewUrl, WebviewWindowBuilder};

use crate::config::{self, Config};
use crate::icon;
use crate::inject;
use crate::window::AppWindow;
use crate::window::{show_when_loaded, theme_color, PAGE_BG};

/// Contains `:`, so it can't clash with an app window label (`[A-Za-z0-9_-]+`).
pub const LAUNCHER_LABEL: &str = "weboverlay:launcher";

const RECENT_MAX: usize = 10;

/// Content of a config created from the launcher.
const NEW_CONFIG: &str = r##"# WebOverlay config: one [[app]] table per window. Script/style paths are relative to this file.
# Uncomment and adapt the example below; every key except `name` and `url` is optional.

# [[app]]
# name = "example"                    # [A-Za-z0-9_-]+, unique (window label, profile dir)
# url = "https://example.com/"        # start page
# title = "Example"                   # window title (default: page title)
# width = 1200                        # default 1200
# height = 800                        # default 800
# maximized = false                   # width/height still apply when restored
# zoom = 1.0                          # initial page zoom (0.3 to 3.0), also the header's zoom reset
# user_agent = "Mozilla/5.0 ..."      # default: WebView user agent
# allowed_hosts = ["example.com", "*.example.com"]   # default: host of `url`; others open in the system browser
# styles = [
#   { path = "example/style.css", matches = ["https://example.com/*"] },   # matches: default every page
# ]
# scripts = [
#   { path = "example/early.js", run_at = "document-start" },              # document-start | document-end (default) | document-idle
#   { path = "example/main.js", run_at = "document-end", matches = ["https://example.com/*"] },
# ]
# [app.header]
# color = "#1e3a8a"                   # background (#rgb / #rrggbb), default: system theme
# text_color = "#ffffff"              # default: light or dark, following `color`
# hide = ["zoom", "edit"]             # back | forward | reload | home | favicon | title | url | zoom | edit | external
"##;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    config: PathBuf,
    app: String,
    title: String,
    url: String,
    /// Unix time in seconds.
    opened: u64,
}

#[derive(Serialize)]
struct RecentItem {
    #[serde(flatten)]
    entry: RecentEntry,
    /// The config still parses and defines the app.
    available: bool,
    /// Cached favicon (`data:` URL), filled by `launcher_list`.
    icon: Option<String>,
}

#[derive(Serialize)]
struct AppEntry {
    name: String,
    title: String,
    url: String,
    /// Cached favicon (`data:` URL), filled by `launcher_list`.
    icon: Option<String>,
}

#[derive(Serialize)]
struct ConfigEntry {
    /// `user`, `folder` or `added` (created from the launcher).
    kind: &'static str,
    path: PathBuf,
    exists: bool,
    error: Option<String>,
    apps: Vec<AppEntry>,
}

#[derive(Serialize)]
pub struct Sources {
    recent: Vec<RecentItem>,
    configs: Vec<ConfigEntry>,
}

pub fn open(handle: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(
        handle,
        LAUNCHER_LABEL,
        WebviewUrl::App("launcher.html".into()),
    )
    .title("WebOverlay")
    .inner_size(600.0, 400.0)
    .min_inner_size(420.0, 240.0)
    .decorations(false)
    .visible(false)
    .on_page_load(show_when_loaded)
    .build()?;
    let _ = window.set_background_color(Some(theme_color(&window.as_ref().window(), PAGE_BG)));
    icon::set_launcher_id(&window.as_ref().window());
    Ok(())
}

fn recent_path() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("weboverlay")
            .join("recent.json"),
    )
}

/// Configs created from the launcher.
fn added_path() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("weboverlay")
            .join("configs.json"),
    )
}

/// The JSON list at `path` (empty when missing or invalid).
fn load_list<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_list<T: Serialize>(path: &Path, list: &[T]) -> Result<(), String> {
    path.parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(path, serde_json::to_string_pretty(list).unwrap_or_default()))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// `new` first, then the `old` entries not replaced by one of them, at most `RECENT_MAX`.
fn merge_recent(old: Vec<RecentEntry>, new: Vec<RecentEntry>) -> Vec<RecentEntry> {
    let mut list = new;
    for entry in old {
        if !list
            .iter()
            .any(|e| e.config == entry.config && e.app == entry.app)
        {
            list.push(entry);
        }
    }
    list.truncate(RECENT_MAX);
    list
}

/// `list` without app `app` of `config`.
fn remove_recent(list: Vec<RecentEntry>, config: &Path, app: &str) -> Vec<RecentEntry> {
    list.into_iter()
        .filter(|e| !(e.config == config && e.app == app))
        .collect()
}

/// Adds the opened apps to the recent list (errors are only logged).
pub fn record(config_path: &Path, apps: &[Arc<AppWindow>]) {
    let Some(path) = recent_path() else {
        return;
    };
    // Not `canonicalize`: it yields `\\?\` paths on Windows.
    let config = std::path::absolute(config_path).unwrap_or_else(|_| config_path.to_owned());
    let opened = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let new = apps
        .iter()
        .map(|app| RecentEntry {
            config: config.clone(),
            app: app.config.name.clone(),
            title: app.config.title.clone().unwrap_or(app.config.name.clone()),
            url: app.config.url.to_string(),
            opened,
        })
        .collect();
    let list = merge_recent(load_list(&path), new);
    if let Err(e) = save_list(&path, &list) {
        eprintln!("weboverlay: {e}");
    }
}

fn config_entry(kind: &'static str, path: PathBuf) -> ConfigEntry {
    let mut entry = ConfigEntry {
        kind,
        exists: path.is_file(),
        path,
        error: None,
        apps: Vec::new(),
    };
    if !entry.exists {
        return entry;
    }
    match Config::load(&entry.path) {
        Ok(config) => {
            entry.apps = config
                .apps
                .into_iter()
                .map(|app| AppEntry {
                    title: app.title.unwrap_or(app.name.clone()),
                    name: app.name,
                    url: app.url.to_string(),
                    icon: None,
                })
                .collect();
        }
        Err(e) => entry.error = Some(format!("{e:#}")),
    }
    entry
}

/// `locals`: (kind, path) of the local configs; missing `folder` configs are left out.
fn sources_from(recent: Vec<RecentEntry>, locals: Vec<(&'static str, PathBuf)>) -> Sources {
    let mut configs: Vec<ConfigEntry> = Vec::new();
    for (kind, path) in locals {
        if configs.iter().any(|c| c.path == path) || (kind == "folder" && !path.is_file()) {
            continue;
        }
        configs.push(config_entry(kind, path));
    }
    let mut loaded: HashMap<PathBuf, Option<Config>> = HashMap::new();
    let recent = recent
        .into_iter()
        .map(|entry| {
            let config = loaded
                .entry(entry.config.clone())
                .or_insert_with(|| Config::load(&entry.config).ok());
            let available = config
                .as_ref()
                .is_some_and(|c| c.find(&entry.app).is_some());
            RecentItem {
                entry,
                available,
                icon: None,
            }
        })
        .collect();
    Sources { recent, configs }
}

fn sources() -> Sources {
    let recent = recent_path().map(|p| load_list(&p)).unwrap_or_default();
    let mut locals = Vec::new();
    if let Some(path) = config::portable_path() {
        locals.push(("folder", path));
    }
    if let Ok(path) = config::user_path() {
        locals.push(("user", path));
    }
    let added: Vec<PathBuf> = added_path().map(|p| load_list(&p)).unwrap_or_default();
    locals.extend(added.into_iter().map(|path| ("added", path)));
    sources_from(recent, locals)
}

impl Sources {
    /// `app` of `config` is listed (any listed app of `config` when `app` is `None`).
    fn lists(&self, config: &Path, app: Option<&str>) -> bool {
        let in_recent = self.recent.iter().any(|item| {
            item.available
                && item.entry.config == config
                && app.is_none_or(|app| item.entry.app == app)
        });
        let in_configs = self.configs.iter().any(|entry| {
            entry.path == config
                && entry
                    .apps
                    .iter()
                    .any(|a| app.is_none_or(|app| a.name == app))
        });
        in_recent || in_configs
    }

    fn has_config(&self, config: &Path) -> bool {
        self.configs.iter().any(|entry| entry.path == config)
    }

    fn with_icons(mut self) -> Self {
        for item in &mut self.recent {
            item.icon = icon::cached(&item.entry.config, &item.entry.app);
        }
        for entry in &mut self.configs {
            for app in &mut entry.apps {
                app.icon = icon::cached(&entry.path, &app.name);
            }
        }
        self
    }
}

fn check_caller(webview: &Webview) -> Result<(), String> {
    if webview.label() == LAUNCHER_LABEL {
        Ok(())
    } else {
        Err("not allowed".into())
    }
}

/// `path` first, then the other `list` entries.
fn add_config(list: Vec<PathBuf>, path: PathBuf) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = list.into_iter().filter(|p| *p != path).collect();
    list.insert(0, path);
    list
}

/// Starts this executable with `args`; the child is reaped from a thread.
fn spawn(args: Vec<OsString>) -> Result<(), String> {
    let exe = config::exe_path().ok_or("cannot determine the executable path")?;
    let mut child = Command::new(&exe)
        .args(args)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[tauri::command]
pub fn launcher_list(webview: Webview) -> Result<Sources, String> {
    check_caller(&webview)?;
    Ok(sources().with_icons())
}

/// Downloads the favicon of a listed app without cached icon (see `icon::fetch_cached`).
#[tauri::command]
pub async fn launcher_icon(
    webview: Webview,
    config: PathBuf,
    app: String,
) -> Result<Option<String>, String> {
    check_caller(&webview)?;
    tauri::async_runtime::spawn_blocking(move || {
        if !sources().lists(&config, Some(&app)) {
            return Err(format!("{app} is not a listed app"));
        }
        let loaded = Config::load(&config).map_err(|e| format!("{e:#}"))?;
        let entry = loaded.find(&app).ok_or("unknown app")?;
        Ok(icon::fetch_cached(
            &config,
            &app,
            &entry.url,
            entry.user_agent.as_deref(),
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens `app` of `config` (all its apps when `None`) in a new process.
#[tauri::command]
pub fn launcher_open(webview: Webview, config: PathBuf, app: Option<String>) -> Result<(), String> {
    check_caller(&webview)?;
    if !sources().lists(&config, app.as_deref()) {
        return Err(format!(
            "{} is not a listed app",
            app.as_deref().unwrap_or("this")
        ));
    }
    // The new instance would exit silently on these errors: report them here instead.
    let loaded = Config::load(&config).map_err(|e| format!("{e:#}"))?;
    for entry in &loaded.apps {
        if app.as_ref().is_none_or(|app| &entry.name == app) {
            inject::build_scripts(entry).map_err(|e| format!("{e:#}"))?;
        }
    }
    let mut args = vec![OsString::from("--config"), config.into_os_string()];
    args.extend(app.map(OsString::from));
    spawn(args)
}

fn spawn_editor(config: PathBuf) -> Result<(), String> {
    spawn(vec![
        "--edit".into(),
        "--config".into(),
        config.into_os_string(),
    ])
}

/// Opens the editor on a listed local config (which may not exist yet) in a new process.
#[tauri::command]
pub fn launcher_edit(webview: Webview, config: PathBuf) -> Result<(), String> {
    check_caller(&webview)?;
    if !sources().has_config(&config) {
        return Err(format!("{} is not a listed config", config.display()));
    }
    spawn_editor(config)
}

/// Asks for a config path with a native "save as" dialog, creates the file from `NEW_CONFIG`
/// (an existing file is kept as is), adds it to the launcher and opens it in the editor.
/// Returns `None` when the dialog is cancelled.
#[tauri::command]
pub async fn launcher_create(webview: Webview) -> Result<Option<PathBuf>, String> {
    check_caller(&webview)?;
    let list_path = added_path().ok_or("cannot determine the local data directory")?;
    let window = webview.window();
    let start = config::user_path()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let (tx, rx) = std::sync::mpsc::channel();
    // Like tauri-plugin-dialog: the dialog is built on the main (GTK) thread, awaited elsewhere.
    webview
        .run_on_main_thread(move || {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("New config")
                .set_file_name("config.toml")
                .add_filter("TOML", &["toml"])
                .set_parent(&window);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            let picked = dialog.save_file();
            std::thread::spawn(move || {
                let _ = tx.send(tauri::async_runtime::block_on(picked));
            });
        })
        .map_err(|e| e.to_string())?;
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| e.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = std::path::absolute(picked.path()).map_err(|e| e.to_string())?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(&path, NEW_CONFIG)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    save_list(&list_path, &add_config(load_list(&list_path), path.clone()))?;
    spawn_editor(path.clone())?;
    Ok(Some(path))
}

/// Removes an added config from the launcher (the file is kept).
#[tauri::command]
pub fn launcher_forget(webview: Webview, config: PathBuf) -> Result<(), String> {
    check_caller(&webview)?;
    let list_path = added_path().ok_or("cannot determine the local data directory")?;
    let mut list: Vec<PathBuf> = load_list(&list_path);
    let len = list.len();
    list.retain(|p| *p != config);
    if list.len() == len {
        return Err(format!("{} is not an added config", config.display()));
    }
    save_list(&list_path, &list)
}

/// Window controls of the undecorated launcher (its page draws the title bar).
/// Returns whether the window is maximized.
#[tauri::command]
pub fn launcher_window(webview: Webview, action: String) -> Result<bool, String> {
    check_caller(&webview)?;
    let window = webview.window();
    let result = match action.as_str() {
        "state" => Ok(()),
        "minimize" => window.minimize(),
        "toggle_maximize" => match window.is_maximized() {
            Ok(true) => window.unmaximize(),
            _ => window.maximize(),
        },
        "close" => window.close(),
        "start_dragging" => window.start_dragging(),
        _ => return Err(format!("unknown action {action:?}")),
    };
    result.map_err(|e| e.to_string())?;
    window.is_maximized().map_err(|e| e.to_string())
}

/// Removes an app from the recent list.
#[tauri::command]
pub fn launcher_forget_recent(
    webview: Webview,
    config: PathBuf,
    app: String,
) -> Result<(), String> {
    check_caller(&webview)?;
    let path = recent_path().ok_or("cannot determine the local data directory")?;
    let list: Vec<RecentEntry> = load_list(&path);
    let len = list.len();
    let list = remove_recent(list, &config, &app);
    if list.len() == len {
        return Err(format!("{app} is not a recent app"));
    }
    save_list(&path, &list)
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

    fn entry(config: &str, app: &str) -> RecentEntry {
        RecentEntry {
            config: config.into(),
            app: app.into(),
            title: app.into(),
            url: "https://example.com/".into(),
            opened: 0,
        }
    }

    #[test]
    fn merges_recent() {
        let old = vec![entry("a", "x"), entry("a", "y"), entry("b", "x")];
        let list = merge_recent(old, vec![entry("a", "y"), entry("c", "z")]);
        let keys: Vec<_> = list
            .iter()
            .map(|e| (e.config.clone(), &e.app[..]))
            .collect();
        assert_eq!(
            keys,
            [
                ("a".into(), "y"),
                ("c".into(), "z"),
                ("a".into(), "x"),
                ("b".into(), "x")
            ]
        );
        let many = (0..20).map(|i| entry("a", &i.to_string())).collect();
        assert_eq!(merge_recent(many, Vec::new()).len(), RECENT_MAX);
    }

    #[test]
    fn removes_recent() {
        let list = vec![entry("a", "x"), entry("a", "y"), entry("b", "x")];
        let list = remove_recent(list, Path::new("a"), "x");
        let keys: Vec<_> = list
            .iter()
            .map(|e| (e.config.clone(), &e.app[..]))
            .collect();
        assert_eq!(keys, [("a".into(), "y"), ("b".into(), "x")]);
        assert_eq!(remove_recent(list, Path::new("c"), "x").len(), 2);
    }

    #[test]
    fn adds_config() {
        let list = add_config(vec!["a".into(), "b".into()], "b".into());
        assert_eq!(list, [PathBuf::from("b"), PathBuf::from("a")]);
        assert_eq!(add_config(Vec::new(), "c".into()), [PathBuf::from("c")]);
        assert!(Config::parse(NEW_CONFIG).unwrap().apps.is_empty());
    }

    #[test]
    fn new_config_example_parses() {
        let uncommented: String = NEW_CONFIG
            .lines()
            .skip(3)
            .map(|line| format!("{}\n", line.strip_prefix("# ").unwrap_or(line)))
            .collect();
        let config = Config::parse(&uncommented).unwrap();
        let app = &config.apps[0];
        assert_eq!(app.name, "example");
        assert_eq!(app.scripts.len(), 2);
        assert_eq!(app.styles.len(), 1);
        assert_eq!(app.header.hide.len(), 2);
    }

    #[test]
    fn lists_sources() {
        let dir = temp_dir("launcher");
        let good = dir.join("good.toml");
        fs::write(
            &good,
            "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\ntitle = \"A\"",
        )
        .unwrap();
        let bad = dir.join("bad.toml");
        fs::write(&bad, "[[app]]\nname = \"a b\"").unwrap();
        let missing = dir.join("missing.toml");

        let sources = sources_from(
            vec![
                entry(good.to_str().unwrap(), "a"),
                entry(good.to_str().unwrap(), "gone"),
            ],
            vec![
                ("folder", missing.clone()),
                ("user", good.clone()),
                ("user", bad.clone()),
                ("user", good.clone()),
                ("added", good.clone()),
            ],
        );
        let paths: Vec<_> = sources.configs.iter().map(|c| c.path.clone()).collect();
        assert_eq!(paths, [good.clone(), bad.clone()]);
        assert_eq!(sources.configs[0].apps[0].title, "A");
        assert!(sources.configs[1].error.is_some());
        assert!(sources.recent[0].available);
        assert!(!sources.recent[1].available);

        assert!(sources.lists(&good, Some("a")));
        assert!(sources.lists(&good, None));
        assert!(!sources.lists(&good, Some("gone")));
        assert!(!sources.lists(&bad, None));
        assert!(!sources.lists(&missing, None));
        assert!(sources.has_config(&bad));
        assert!(!sources.has_config(&missing));

        let user_missing = sources_from(
            Vec::new(),
            vec![("user", missing.clone()), ("added", missing.clone())],
        );
        assert_eq!(user_missing.configs.len(), 1);
        assert!(!user_missing.configs[0].exists);
        assert!(user_missing.has_config(&missing));
        fs::remove_dir_all(dir).unwrap();
    }
}
