//! Vault: logins saved in the system keyring (Secret Service on Linux, Credential Manager on
//! Windows), entered by hand in the vault window (`dist/vault.html`) and filled into the login
//! forms of the content, from the header `vault` button or on page load (`[app.vault] fill`).
//!
//! One keyring entry per login (service `SERVICE`, user `<app id>|<origin>|<username>`). Keyring
//! stores can't list their entries, so the logins of an app are listed in a non-secret index,
//! `<local data dir>/weboverlay/vault/<app id>.json` (origins and usernames, never passwords).
//!
//! Passwords reach a page only through `fill_script`, evaluated in the content after its origin
//! was checked against the login's (again by the script itself, against a navigation race).
//! Keyring calls may block (D-Bus, unlock prompt): they run off the main thread.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Url, Webview, WebviewUrl, WebviewWindowBuilder};

use crate::config::VaultFill;
use crate::header;
use crate::icon;
use crate::window::{self, show_when_loaded, theme_color, AppWindow, PAGE_BG};

/// Contains `:`, so it can't clash with an app window label (`[A-Za-z0-9_-]+`).
pub const VAULT_LABEL: &str = "weboverlay:vault";

const SERVICE: &str = "dev.blet.weboverlay";

/// Serializes the index read-modify-write cycles.
static INDEX: Mutex<()> = Mutex::new(());

/// A saved login (the password stays in the keyring).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    pub origin: String,
    pub username: String,
}

/// `scheme://host[:port]` of an http(s) URL.
pub fn origin_of(url: &str) -> Result<String, String> {
    let url = Url::parse(url.trim()).map_err(|_| format!("invalid URL {url:?}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("the site must be an http(s) URL".into());
    }
    Ok(url.origin().ascii_serialization())
}

/// Sets the keyring store of this OS as the default one, once (retried after a failure: the
/// Secret Service may start later).
fn store() -> Result<(), String> {
    static READY: Mutex<bool> = Mutex::new(false);
    let mut ready = READY.lock().unwrap_or_else(|e| e.into_inner());
    if !*ready {
        #[cfg(target_os = "linux")]
        let store = zbus_secret_service_keyring_store::Store::new();
        #[cfg(windows)]
        let store = windows_native_keyring_store::Store::new();
        #[cfg(not(any(target_os = "linux", windows)))]
        let store: keyring_core::Result<Arc<keyring_core::CredentialStore>> =
            Err(keyring_core::Error::NoDefaultStore);
        let store = store.map_err(|e| format!("system keyring unavailable: {e}"))?;
        keyring_core::set_default_store(store);
        *ready = true;
    }
    Ok(())
}

fn entry(app: &AppWindow, login: &Login) -> Result<keyring_core::Entry, String> {
    store()?;
    let user = format!("{}|{}|{}", app_id(app), login.origin, login.username);
    keyring_core::Entry::new(SERVICE, &user).map_err(keyring_error)
}

fn keyring_error(e: keyring_core::Error) -> String {
    match e {
        keyring_core::Error::NoEntry => "password not found in the system keyring".into(),
        e => format!("system keyring: {e}"),
    }
}

fn app_id(app: &AppWindow) -> String {
    icon::app_id(&app.config_path, &app.config.name)
}

fn index_path(app: &AppWindow) -> Result<PathBuf, String> {
    let dir = dirs::data_local_dir().ok_or("no local data directory")?;
    Ok(dir
        .join("weboverlay")
        .join("vault")
        .join(format!("{}.json", app_id(app))))
}

fn read_index(app: &AppWindow) -> Result<Vec<Login>, String> {
    let path = index_path(app)?;
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn write_index(app: &AppWindow, logins: &[Login]) -> Result<(), String> {
    let path = index_path(app)?;
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(error)?;
    }
    let json = serde_json::to_vec_pretty(logins).expect("logins serialize");
    // Written next to the index, then renamed: never a truncated index.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(error)?;
    fs::rename(&tmp, &path).map_err(error)
}

/// Logins of `app`, sorted by origin then username.
pub fn list(app: &AppWindow) -> Result<Vec<Login>, String> {
    let _guard = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut logins = read_index(app)?;
    logins.sort_by(|a, b| (&a.origin, &a.username).cmp(&(&b.origin, &b.username)));
    Ok(logins)
}

/// Logins of `app` for the origin of `url`.
fn matching(app: &AppWindow, url: &Url) -> Result<Vec<Login>, String> {
    let origin = url.origin().ascii_serialization();
    Ok(list(app)?
        .into_iter()
        .filter(|login| login.origin == origin)
        .collect())
}

/// Saves a login, replacing `previous` (edit) when given. An empty `password` keeps the one of
/// `previous`.
pub fn save(
    app: &AppWindow,
    login: Login,
    password: &str,
    previous: Option<Login>,
) -> Result<(), String> {
    let login = Login {
        origin: origin_of(&login.origin)?,
        username: login.username.trim().to_owned(),
    };
    let _guard = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut logins = read_index(app)?;
    if previous.as_ref() != Some(&login) && logins.contains(&login) {
        return Err(format!(
            "a login for {:?} on {} is already saved",
            login.username, login.origin
        ));
    }
    let password = match (&previous, password) {
        (Some(previous), "") => entry(app, previous)?
            .get_password()
            .map_err(keyring_error)?,
        (None, "") => return Err("the password is empty".into()),
        (_, password) => password.to_owned(),
    };
    entry(app, &login)?
        .set_password(&password)
        .map_err(keyring_error)?;
    if let Some(previous) = previous.filter(|previous| *previous != login) {
        delete_secret(app, &previous)?;
        logins.retain(|l| *l != previous);
    }
    if !logins.contains(&login) {
        logins.push(login);
    }
    write_index(app, &logins)
}

fn delete_secret(app: &AppWindow, login: &Login) -> Result<(), String> {
    match entry(app, login)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(keyring_error(e)),
    }
}

pub fn delete(app: &AppWindow, login: &Login) -> Result<(), String> {
    let _guard = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    delete_secret(app, login)?;
    let mut logins = read_index(app)?;
    logins.retain(|l| l != login);
    write_index(app, &logins)
}

fn password(app: &AppWindow, login: &Login) -> Result<String, String> {
    entry(app, login)?.get_password().map_err(keyring_error)
}

/// Fills `login` into the login form of the content page. Blocking (keyring).
fn fill(content: &Webview, app: &AppWindow, login: &Login) -> Result<(), String> {
    let url = content.url().map_err(|e| e.to_string())?;
    if url.origin().ascii_serialization() != login.origin {
        return Err(format!("the page is not on {}", login.origin));
    }
    let password = password(app, login)?;
    content
        .eval(fill_script(&login.origin, &login.username, &password))
        .map_err(|e| e.to_string())
}

/// Evaluated in the content: fills the first visible password field and the username field
/// before it (or a username-only step of a two-step login), waiting up to 10 s for a form
/// rendered later. Never submits.
fn fill_script(origin: &str, username: &str, password: &str) -> String {
    let json = |s: &str| serde_json::to_string(s).expect("string serializes");
    format!(
        r#"(() => {{
  const origin = {origin}, username = {username}, password = {password};
  if (location.origin !== origin) return;
  const done = new WeakSet();
  const visible = (el) => !el.disabled && !el.readOnly && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const set = (el, value) => {{
    if (done.has(el)) return;
    done.add(el);
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, value);
    el.dispatchEvent(new Event("input", {{ bubbles: true }}));
    el.dispatchEvent(new Event("change", {{ bubbles: true }}));
  }};
  const isUser = (el) => ["text", "email", "tel"].includes(el.type) &&
    (/username|email/i.test(el.autocomplete) || el.type === "email" || /user|login|mail|ident/i.test(el.name + " " + el.id));
  const fill = () => {{
    const inputs = Array.from(document.querySelectorAll("input")).filter(visible);
    const pass = inputs.find((el) => el.type === "password");
    if (pass) {{
      const scope = inputs.filter((el) => el.form === pass.form);
      const before = scope.slice(0, scope.indexOf(pass)).filter((el) => ["text", "email", "tel"].includes(el.type));
      const user = before.find(isUser) || before[before.length - 1];
      if (user && username) set(user, username);
      set(pass, password);
      pass.focus();
      return true;
    }}
    const user = inputs.find(isUser);
    if (user && username) set(user, username);
    return false;
  }};
  try {{
    if (fill()) return;
    const observer = new MutationObserver(() => {{ if (fill()) observer.disconnect(); }});
    observer.observe(document.documentElement, {{ childList: true, subtree: true, attributes: true }});
    setTimeout(() => observer.disconnect(), 10000);
  }} catch (_) {{}}
}})()"#,
        origin = json(origin),
        username = json(username),
        password = json(password),
    )
}

/// `[app.vault] fill = "auto"`: fills the only login saved for the page origin. From a thread.
pub fn autofill(content: &Webview) {
    let handle = content.app_handle();
    let Some(app) = header::app(handle, content.window().label()) else {
        return;
    };
    if app.config.vault.fill != VaultFill::Auto {
        return;
    }
    let content = content.clone();
    std::thread::spawn(move || {
        let Ok(url) = content.url() else { return };
        if let Ok([login]) = matching(&app, &url).as_deref() {
            if let Err(e) = fill(&content, &app, login) {
                eprintln!("weboverlay: vault: {e}");
            }
        }
    });
}

/// Header `vault` button: fills the only login saved for the page origin, else opens the vault
/// window. From a thread.
pub fn button(handle: &AppHandle, content: &Webview, app: Arc<AppWindow>) {
    let handle = handle.clone();
    let content = content.clone();
    std::thread::spawn(move || {
        let logins = content
            .url()
            .map_err(|e| e.to_string())
            .and_then(|url| matching(&app, &url));
        if let Ok([login]) = logins.as_deref() {
            match fill(&content, &app, login) {
                Ok(()) => return,
                Err(e) => eprintln!("weboverlay: vault: {e}"),
            }
        }
        if let Err(e) = open(&handle) {
            eprintln!("weboverlay: cannot open the vault window: {e}");
        }
    });
}

/// Opens the vault window, or focuses it when already open. Not from a sync command (see
/// `editor::open`).
fn open(handle: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = handle.get_webview_window(VAULT_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        // Refreshes the list and the current origin.
        let _ = window.eval("window.vaultRefresh && window.vaultRefresh()");
        return window.set_focus();
    }
    let window =
        WebviewWindowBuilder::new(handle, VAULT_LABEL, WebviewUrl::App("vault.html".into()))
            .title("Vault — WebOverlay")
            .inner_size(520.0, 460.0)
            .min_inner_size(360.0, 260.0)
            .visible(false)
            .on_page_load(show_when_loaded)
            .build()?;
    let _ = window.set_background_color(Some(theme_color(&window.as_ref().window(), PAGE_BG)));
    icon::set_default_taskbar_icon(&window.as_ref().window());
    Ok(())
}

/// The app window of this process (one app per process) and its content webview.
fn current(webview: &Webview) -> Result<(Arc<AppWindow>, Webview), String> {
    if webview.label() != VAULT_LABEL {
        return Err("not allowed".into());
    }
    let handle = webview.app_handle();
    let (label, app) = header::first_app(handle).ok_or("no app window")?;
    let content = handle
        .get_webview(&window::content_label(&label))
        .ok_or("no content webview")?;
    Ok((app, content))
}

/// Runs blocking keyring work off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct VaultView {
    /// Origin of the content page (empty when not http(s)).
    current: String,
    logins: Vec<Login>,
}

#[tauri::command]
pub async fn vault_list(webview: Webview) -> Result<VaultView, String> {
    let (app, content) = current(&webview)?;
    let current = content
        .url()
        .ok()
        .and_then(|url| origin_of(url.as_str()).ok())
        .unwrap_or_default();
    blocking(move || {
        Ok(VaultView {
            current,
            logins: list(&app)?,
        })
    })
    .await
}

#[tauri::command]
pub async fn vault_save(
    webview: Webview,
    login: Login,
    password: String,
    previous: Option<Login>,
) -> Result<(), String> {
    let (app, _) = current(&webview)?;
    blocking(move || save(&app, login, &password, previous)).await
}

#[tauri::command]
pub async fn vault_delete(webview: Webview, login: Login) -> Result<(), String> {
    let (app, _) = current(&webview)?;
    blocking(move || delete(&app, &login)).await
}

#[tauri::command]
pub async fn vault_password(webview: Webview, login: Login) -> Result<String, String> {
    let (app, _) = current(&webview)?;
    blocking(move || password(&app, &login)).await
}

#[tauri::command]
pub async fn vault_fill(webview: Webview, login: Login) -> Result<(), String> {
    let (app, content) = current(&webview)?;
    blocking(move || fill(&content, &app, &login)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        assert_eq!(origin_of("https://a.com/login?x").unwrap(), "https://a.com");
        assert_eq!(
            origin_of(" HTTP://A.com:8080/").unwrap(),
            "http://a.com:8080"
        );
        assert_eq!(origin_of("https://a.com:443").unwrap(), "https://a.com");
        assert!(origin_of("a.com").is_err());
        assert!(origin_of("file:///etc/passwd").is_err());
        assert!(origin_of("javascript:alert(1)").is_err());
    }

    #[test]
    fn fill_script_embeds_json_strings() {
        let js = fill_script("https://a.com", "me\"</script>", "p\\w'\n\u{2028}");
        assert!(js.contains(r#"const origin = "https://a.com", "#), "{js}");
        assert!(js.contains(r#"username = "me\"</script>""#), "{js}");
        assert!(js.contains(r#"password = "p\\w'\n"#), "{js}");
        assert!(js.contains("if (location.origin !== origin) return;"));
    }

    #[test]
    fn login_json() {
        let login = Login {
            origin: "https://a.com".into(),
            username: "me".into(),
        };
        let json = serde_json::to_string(&[&login]).unwrap();
        assert_eq!(json, r#"[{"origin":"https://a.com","username":"me"}]"#);
        let back: Vec<Login> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, [login]);
    }
}
