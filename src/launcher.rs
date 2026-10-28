//! Launcher window (`dist/launcher.html`), shown when started without app nor `--config`:
//! recent apps plus the apps of the user and portable configs, each started in a new process.
//! Only the launcher webview may call its commands, and only on the listed configs/apps.

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
use crate::inject;
use crate::window::AppWindow;

/// Contains `:`, so it can't clash with an app window label (`[A-Za-z0-9_-]+`).
pub const LAUNCHER_LABEL: &str = "weboverlay:launcher";

const RECENT_MAX: usize = 10;

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
}

#[derive(Serialize)]
struct AppEntry {
    name: String,
    title: String,
    url: String,
}

#[derive(Serialize)]
struct ConfigEntry {
    /// `user` or `folder`.
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
    WebviewWindowBuilder::new(
        handle,
        LAUNCHER_LABEL,
        WebviewUrl::App("launcher.html".into()),
    )
    .title("WebOverlay")
    .inner_size(760.0, 560.0)
    .build()?;
    Ok(())
}

fn recent_path() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("weboverlay")
            .join("recent.json"),
    )
}

fn load_recent(path: &Path) -> Vec<RecentEntry> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
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
    let list = merge_recent(load_recent(&path), new);
    let result = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| {
            fs::write(
                &path,
                serde_json::to_string_pretty(&list).unwrap_or_default(),
            )
        });
    if let Err(e) = result {
        eprintln!("weboverlay: cannot write {}: {e}", path.display());
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
            RecentItem { entry, available }
        })
        .collect();
    Sources { recent, configs }
}

fn sources() -> Sources {
    let recent = recent_path().map(|p| load_recent(&p)).unwrap_or_default();
    let mut locals = Vec::new();
    if let Some(path) = config::portable_path() {
        locals.push(("folder", path));
    }
    if let Ok(path) = config::user_path() {
        locals.push(("user", path));
    }
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
}

fn check_caller(webview: &Webview) -> Result<(), String> {
    if webview.label() == LAUNCHER_LABEL {
        Ok(())
    } else {
        Err("not allowed".into())
    }
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
    Ok(sources())
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

/// Opens the editor on a listed local config (which may not exist yet) in a new process.
#[tauri::command]
pub fn launcher_edit(webview: Webview, config: PathBuf) -> Result<(), String> {
    check_caller(&webview)?;
    if !sources().has_config(&config) {
        return Err(format!("{} is not a listed config", config.display()));
    }
    spawn(vec![
        "--edit".into(),
        "--config".into(),
        config.into_os_string(),
    ])
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

        let user_missing = sources_from(Vec::new(), vec![("user", missing.clone())]);
        assert!(!user_missing.configs[0].exists);
        assert!(user_missing.has_config(&missing));
        fs::remove_dir_all(dir).unwrap();
    }
}
