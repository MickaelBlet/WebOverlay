use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
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
    /// Initial page zoom (1.0 = 100%), also the target of the header's zoom reset.
    #[serde(default = "default_zoom")]
    pub zoom: f64,
    pub user_agent: Option<String>,
    /// Image file used as window / taskbar / launcher icon instead of the site favicon.
    pub icon: Option<PathBuf>,
    /// Host wildcards allowed inside the window; anything else opens in the system browser.
    /// Defaults to the host of `url`.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Extensions (`ica`, `tar.gz`) of downloads opened with their default app, without the
    /// save dialog. Stored lowercase, without the leading dot.
    #[serde(default)]
    pub open_downloads: Vec<String>,
    #[serde(default)]
    pub scripts: Vec<ScriptEntry>,
    #[serde(default)]
    pub styles: Vec<StyleEntry>,
    #[serde(default)]
    pub header: HeaderConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderConfig {
    /// Background color; the text color follows its luminance unless `text_color` is set.
    pub color: Option<HexColor>,
    pub text_color: Option<HexColor>,
    /// Header items not shown.
    #[serde(default)]
    pub hide: Vec<HeaderItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeaderItem {
    Back,
    Forward,
    Reload,
    Home,
    Favicon,
    Title,
    Url,
    Zoom,
    Downloads,
    Edit,
    External,
    Devtools,
}

/// `#rgb` or `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct HexColor(pub u8, pub u8, pub u8);

impl TryFrom<String> for HexColor {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let error = || format!("invalid color {text:?}, expected #rgb or #rrggbb");
        let hex = text.strip_prefix('#').ok_or_else(error)?;
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(error());
        }
        let channel = |s: &str| u8::from_str_radix(s, 16).map_err(|_| error());
        match hex.len() {
            3 => {
                let short = |i: usize| channel(&hex[i..=i]).map(|v| v * 17);
                Ok(Self(short(0)?, short(1)?, short(2)?))
            }
            6 => Ok(Self(
                channel(&hex[0..2])?,
                channel(&hex[2..4])?,
                channel(&hex[4..6])?,
            )),
            _ => Err(error()),
        }
    }
}

impl HexColor {
    /// WCAG relative luminance below the black/white contrast midpoint.
    pub fn is_dark(self) -> bool {
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let l = 0.2126 * linear(self.0) + 0.7152 * linear(self.1) + 0.0722 * linear(self.2);
        l < 0.179
    }

    pub fn rgba(self, alpha: f64) -> String {
        format!("rgba({}, {}, {}, {alpha})", self.0, self.1, self.2)
    }
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

fn default_zoom() -> f64 {
    1.0
}

pub const ZOOM_MIN: f64 = 0.3;
pub const ZOOM_MAX: f64 = 3.0;

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
    /// Loads the config and resolves relative script/style/icon paths against the config directory.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("cannot read config {}", path.display()))?;
        Self::parse_at(&text, path).with_context(|| format!("invalid config {}", path.display()))
    }

    /// Parses `text` as the config file at `path` (relative script/style/icon paths resolved).
    pub fn parse_at(text: &str, path: &Path) -> Result<Self> {
        let mut config = Self::parse(text)?;
        let base = path.parent().unwrap_or(Path::new("."));
        for app in &mut config.apps {
            if let Some(icon) = &mut app.icon {
                *icon = base.join(&*icon);
            }
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
            if !(ZOOM_MIN..=ZOOM_MAX).contains(&app.zoom) {
                bail!(
                    "app {:?}: zoom must be between {ZOOM_MIN} and {ZOOM_MAX}",
                    app.name
                );
            }
            if !matches!(app.url.scheme(), "http" | "https") {
                bail!("app {:?}: url must be http(s)", app.name);
            }
            for ext in &mut app.open_downloads {
                let normalized = ext.trim().trim_start_matches('.').to_lowercase();
                if normalized.is_empty() || normalized.contains(['/', '\\', '*']) {
                    bail!(
                        "app {:?}: open_downloads entry {ext:?} must be a file extension, \
                         such as \"ica\"",
                        app.name
                    );
                }
                *ext = normalized;
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
    /// A download named `file_name` is opened without the save dialog (`open_downloads`).
    pub fn opens_download(&self, file_name: &str) -> bool {
        let name = file_name.to_lowercase();
        self.open_downloads
            .iter()
            .any(|ext| name.len() > ext.len() + 1 && name.ends_with(&format!(".{ext}")))
    }

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
        assert_eq!(app.zoom, 1.0);
    }

    #[test]
    fn resolves_paths_against_config_dir() {
        let config = Config::parse_at(
            r#"
            [[app]]
            name = "demo"
            url = "https://example.com/"
            icon = "icons/demo.png"
            scripts = [{ path = "a.js" }]
            "#,
            Path::new("/conf/config.toml"),
        )
        .unwrap();
        let app = config.find("demo").unwrap();
        assert_eq!(app.icon.as_deref(), Some(Path::new("/conf/icons/demo.png")));
        assert_eq!(app.scripts[0].path, Path::new("/conf/a.js"));
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
    fn zoom_range() {
        let app = |zoom: &str| {
            Config::parse(&format!(
                "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\nzoom = {zoom}"
            ))
        };
        assert_eq!(app("1.25").unwrap().apps[0].zoom, 1.25);
        assert_eq!(app("2").unwrap().apps[0].zoom, 2.0);
        assert!(app("0.1").is_err());
        assert!(app("5.0").is_err());
        assert!(app("nan").is_err());
    }

    #[test]
    fn rejects_non_http_url() {
        let text = r#"[[app]]
            name = "a"
            url = "file:///etc/passwd""#;
        assert!(Config::parse(text).is_err());
    }

    #[test]
    fn open_downloads_extensions() {
        let text = "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\nopen_downloads = [\"ICA\", \".tar.gz\"]";
        let config = Config::parse(text).unwrap();
        let app = &config.apps[0];
        assert_eq!(app.open_downloads, ["ica", "tar.gz"]);
        assert!(app.opens_download("launch.ica"));
        assert!(app.opens_download("Launch.ICA"));
        assert!(app.opens_download("a.tar.gz"));
        assert!(!app.opens_download("ica"));
        assert!(!app.opens_download(".ica"));
        assert!(!app.opens_download("launch.ica.txt"));
        assert!(!app.opens_download("a.gz"));
        for bad in ["", ".", "*.ica", "a/b"] {
            let text = format!(
                "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\nopen_downloads = [{bad:?}]"
            );
            assert!(Config::parse(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_header() {
        let config = Config::parse(
            r##"[[app]]
            name = "a"
            url = "https://a.com"
            [app.header]
            color = "#1E3a8a"
            text_color = "#fff"
            hide = ["zoom", "edit"]"##,
        )
        .unwrap();
        let header = &config.apps[0].header;
        assert_eq!(header.color, Some(HexColor(0x1e, 0x3a, 0x8a)));
        assert_eq!(header.text_color, Some(HexColor(255, 255, 255)));
        assert_eq!(header.hide, [HeaderItem::Zoom, HeaderItem::Edit]);
        assert!(HexColor(0x1e, 0x3a, 0x8a).is_dark());
        assert!(!HexColor(0xf3, 0xf4, 0xf6).is_dark());
        for bad in ["red", "#12", "#12345g", "123456", "#+1+2+3"] {
            let text = format!(
                "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\nheader = {{ color = \"{bad}\" }}"
            );
            assert!(Config::parse(&text).is_err(), "{bad}");
        }
        let text =
            "[[app]]\nname = \"a\"\nurl = \"https://a.com\"\nheader = { hide = [\"close\"] }";
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
