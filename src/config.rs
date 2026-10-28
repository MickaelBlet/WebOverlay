use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, rename = "app")]
    pub apps: Vec<AppConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub name: String,
    pub url: Url,
    pub title: Option<String>,
    #[serde(default = "default_width")]
    pub width: f64,
    #[serde(default = "default_height")]
    pub height: f64,
    /// Start maximized (`width`/`height` still apply when restored).
    #[serde(default)]
    pub maximized: bool,
    pub user_agent: Option<String>,
    /// Host wildcards allowed inside the window; anything else opens in the system browser.
    /// Defaults to the host of `url`.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub scripts: Vec<ScriptEntry>,
    #[serde(default)]
    pub styles: Vec<StyleEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptEntry {
    pub path: PathBuf,
    #[serde(default)]
    pub run_at: RunAt,
    /// URL wildcards matched against `location.href`; empty means every page.
    #[serde(default)]
    pub matches: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleEntry {
    pub path: PathBuf,
    #[serde(default)]
    pub matches: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(clippy::enum_variant_names)] // mirrors the userscript `run_at` values
pub enum RunAt {
    DocumentStart,
    #[default]
    DocumentEnd,
    DocumentIdle,
}

fn default_width() -> f64 {
    1200.0
}

fn default_height() -> f64 {
    800.0
}

/// `config.toml` next to the executable (portable mode) if present, else the user config dir.
pub fn default_path() -> Result<PathBuf> {
    match portable_path() {
        Some(path) if path.is_file() => Ok(path),
        _ => user_path(),
    }
}

/// `config.toml` next to the executable (may not exist).
pub fn portable_path() -> Option<PathBuf> {
    exe_dir().map(|dir| dir.join("config.toml"))
}

/// `<config dir>/weboverlay/config.toml` (may not exist).
pub fn user_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("cannot determine the user config directory")?;
    Ok(dir.join("weboverlay").join("config.toml"))
}

fn exe_dir() -> Option<PathBuf> {
    exe_path()?.parent().map(Path::to_path_buf)
}

/// The executable the user launched (the `.AppImage` file itself for AppImages).
pub fn exe_path() -> Option<PathBuf> {
    // Inside an AppImage the executable lives in a temporary mount; use the image's location.
    match std::env::var_os("APPIMAGE") {
        Some(appimage) => Some(PathBuf::from(appimage)),
        None => std::env::current_exe().ok(),
    }
}

impl Config {
    /// Loads the config and resolves relative script/style paths against the config directory.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("cannot read config {}", path.display()))?;
        Self::parse_at(&text, path).with_context(|| format!("invalid config {}", path.display()))
    }

    /// Parses `text` as the config file at `path` (relative script/style paths resolved).
    pub fn parse_at(text: &str, path: &Path) -> Result<Self> {
        let mut config = Self::parse(text)?;
        let base = path.parent().unwrap_or(Path::new("."));
        for app in &mut config.apps {
            for script in &mut app.scripts {
                script.path = base.join(&script.path);
            }
            for style in &mut app.styles {
                style.path = base.join(&style.path);
            }
        }
        Ok(config)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(text)?;
        let mut names = HashSet::new();
        for app in &mut config.apps {
            if app.name.is_empty()
                || !app
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                bail!("app name {:?} must match [A-Za-z0-9_-]+", app.name);
            }
            if !names.insert(app.name.clone()) {
                bail!("duplicate app name {:?}", app.name);
            }
            if !matches!(app.url.scheme(), "http" | "https") {
                bail!("app {:?}: url must be http(s)", app.name);
            }
            if app.allowed_hosts.is_empty() {
                if let Some(host) = app.url.host_str() {
                    app.allowed_hosts.push(host.to_owned());
                }
            }
        }
        Ok(config)
    }

    pub fn find(&self, name: &str) -> Option<&AppConfig> {
        self.apps.iter().find(|app| app.name == name)
    }
}

impl AppConfig {
    pub fn is_host_allowed(&self, host: &str) -> bool {
        self.allowed_hosts.iter().any(|pattern| {
            // `*.example.com` also covers the bare `example.com`.
            pattern.strip_prefix("*.") == Some(host) || wildcard_match(pattern, host)
        })
    }
}

/// Case-insensitive match where `*` stands for any sequence of characters.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == '*' {
            backtrack = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some((bp, bt)) = backtrack {
            p = bp + 1;
            t = bt + 1;
            backtrack = Some((bp, bt + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_app() {
        let config = Config::parse(
            r#"
            [[app]]
            name = "demo"
            url = "https://example.com/"
            scripts = [{ path = "a.js", run_at = "document-start", matches = ["https://example.com/*"] }]
            styles = [{ path = "a.css" }]
            "#,
        )
        .unwrap();
        let app = config.find("demo").unwrap();
        assert_eq!(app.allowed_hosts, ["example.com"]);
        assert_eq!(app.scripts[0].run_at, RunAt::DocumentStart);
        assert_eq!(app.width, 1200.0);
        assert!(!app.maximized);
    }

    #[test]
    fn rejects_bad_names_and_duplicates() {
        let bad = r#"[[app]]
            name = "a b"
            url = "https://example.com""#;
        assert!(Config::parse(bad).is_err());
        let dup = r#"[[app]]
            name = "a"
            url = "https://example.com"
            [[app]]
            name = "a"
            url = "https://example.org""#;
        assert!(Config::parse(dup).is_err());
    }

    #[test]
    fn rejects_non_http_url() {
        let text = r#"[[app]]
            name = "a"
            url = "file:///etc/passwd""#;
        assert!(Config::parse(text).is_err());
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("*.google.com", "mail.google.com"));
        assert!(!wildcard_match("*.google.com", "google.com.evil.io"));
        assert!(wildcard_match(
            "https://*/inbox*",
            "https://mail.example/inbox/1"
        ));
        assert!(wildcard_match("a*b*c", "aXXbYYc"));
        assert!(!wildcard_match("a*b*c", "aXXbYY"));
        assert!(wildcard_match("Example.COM", "example.com"));
    }

    #[test]
    fn host_allowed_includes_bare_domain() {
        let config = Config::parse(
            r#"[[app]]
            name = "a"
            url = "https://mail.google.com"
            allowed_hosts = ["*.google.com"]"#,
        )
        .unwrap();
        let app = &config.apps[0];
        assert!(app.is_host_allowed("google.com"));
        assert!(app.is_host_allowed("accounts.google.com"));
        assert!(!app.is_host_allowed("evilgoogle.com"));
    }
}
