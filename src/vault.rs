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
//!
//! `--password-store basic` (Linux, like Chromium's) keeps the passwords in a plaintext file
//! readable by the user only, `<app id>.basic-secrets.json`, with its own index
//! `<app id>.basic.json`: the two stores never mix.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Url, Webview, WebviewUrl, WebviewWindowBuilder};

use crate::config::{Config, VaultFill};
use crate::editor;
use crate::header;
use crate::icon;
use crate::window::{self, show_when_loaded, theme_color, AppWindow, PAGE_BG};

/// Contains `:`, so it can't clash with an app window label (`[A-Za-z0-9_-]+`).
pub const VAULT_LABEL: &str = "weboverlay:vault";

const SERVICE: &str = "dev.blet.weboverlay";

/// Serializes the index read-modify-write cycles.
static INDEX: Mutex<()> = Mutex::new(());

/// `--password-store basic`: passwords in a plaintext file instead of the keyring.
static BASIC: OnceLock<bool> = OnceLock::new();

/// Set once from the command line, before any vault call.
pub fn set_basic(basic: bool) {
    let _ = BASIC.set(basic);
}

pub fn basic() -> bool {
    BASIC.get().copied().unwrap_or(false)
}

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

fn entry(id: &str, login: &Login) -> Result<keyring_core::Entry, String> {
    store()?;
    let user = format!("{id}|{}|{}", login.origin, login.username);
    keyring_core::Entry::new(SERVICE, &user).map_err(keyring_error)
}

fn keyring_error(e: keyring_core::Error) -> String {
    match e {
        keyring_core::Error::NoEntry => "password not found in the system keyring".into(),
        e => format!("system keyring: {e}"),
    }
}

/// Vault id of `app`: its AppUserModelID (config path hash + name).
fn app_id(app: &AppWindow) -> String {
    icon::app_id(&app.config_path, &app.config.name)
}

fn vault_dir() -> Result<PathBuf, String> {
    let dir = dirs::data_local_dir().ok_or("no local data directory")?;
    Ok(dir.join("weboverlay").join("vault"))
}

fn index_path(id: &str) -> Result<PathBuf, String> {
    let name = if basic() {
        format!("{id}.basic.json")
    } else {
        format!("{id}.json")
    };
    Ok(vault_dir()?.join(name))
}

fn read_index(id: &str) -> Result<Vec<Login>, String> {
    read_json(&index_path(id)?)
}

fn write_index(id: &str, logins: &[Login]) -> Result<(), String> {
    write_json(&index_path(id)?, &logins)
}

/// Contents of `path`, default when missing.
fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Writes `value` to `path`, readable by the user only on Unix (the basic store holds
/// passwords).
fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir).map_err(error)?;
    }
    let json = serde_json::to_vec_pretty(value).expect("vault json serializes");
    // Written next to the file, then renamed: never a truncated file.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&tmp).map_err(error)?, &json).map_err(error)?;
    fs::rename(&tmp, path).map_err(error)
}

fn secrets_path(id: &str) -> Result<PathBuf, String> {
    Ok(vault_dir()?.join(format!("{id}.basic-secrets.json")))
}

/// Key of `login` in the basic store.
fn secret_key(login: &Login) -> String {
    format!("{}|{}", login.origin, login.username)
}

fn get_secret(id: &str, login: &Login) -> Result<String, String> {
    if basic() {
        let secrets: BTreeMap<String, String> = read_json(&secrets_path(id)?)?;
        return secrets
            .get(&secret_key(login))
            .cloned()
            .ok_or_else(|| "password not found in the vault file".into());
    }
    entry(id, login)?.get_password().map_err(keyring_error)
}

fn set_secret(id: &str, login: &Login, password: &str) -> Result<(), String> {
    if basic() {
        let path = secrets_path(id)?;
        let mut secrets: BTreeMap<String, String> = read_json(&path)?;
        secrets.insert(secret_key(login), password.to_owned());
        return write_json(&path, &secrets);
    }
    entry(id, login)?
        .set_password(password)
        .map_err(keyring_error)
}

/// Logins of vault `id`, sorted by origin then username.
fn list(id: &str) -> Result<Vec<Login>, String> {
    let _guard = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut logins = read_index(id)?;
    logins.sort_by(|a, b| (&a.origin, &a.username).cmp(&(&b.origin, &b.username)));
    Ok(logins)
}

/// Logins of vault `id` for the origin of `url`.
fn matching(id: &str, url: &Url) -> Result<Vec<Login>, String> {
    let origin = url.origin().ascii_serialization();
    Ok(list(id)?
        .into_iter()
        .filter(|login| login.origin == origin)
        .collect())
}

/// Saves a login, replacing `previous` (edit) when given. An empty `password` keeps the one of
/// `previous`.
fn save(id: &str, login: Login, password: &str, previous: Option<Login>) -> Result<(), String> {
    let login = Login {
        origin: origin_of(&login.origin)?,
        username: login.username.trim().to_owned(),
    };
    let _guard = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut logins = read_index(id)?;
    if previous.as_ref() != Some(&login) && logins.contains(&login) {
        return Err(format!(
            "a login for {:?} on {} is already saved",
            login.username, login.origin
        ));
    }
    let password = match (&previous, password) {
        (Some(previous), "") => get_secret(id, previous)?,
        (None, "") => return Err("the password is empty".into()),
        (_, password) => password.to_owned(),
    };
    set_secret(id, &login, &password)?;
    if let Some(previous) = previous.filter(|previous| *previous != login) {
        delete_secret(id, &previous)?;
        logins.retain(|l| *l != previous);
    }
    if !logins.contains(&login) {
        logins.push(login);
    }
    write_index(id, &logins)
}

fn delete_secret(id: &str, login: &Login) -> Result<(), String> {
    if basic() {
        let path = secrets_path(id)?;
        let mut secrets: BTreeMap<String, String> = read_json(&path)?;
        if secrets.remove(&secret_key(login)).is_some() {
            write_json(&path, &secrets)?;
        }
        return Ok(());
    }
    match entry(id, login)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(keyring_error(e)),
    }
}

fn delete(id: &str, login: &Login) -> Result<(), String> {
    let _guard = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    delete_secret(id, login)?;
    let mut logins = read_index(id)?;
    logins.retain(|l| l != login);
    write_index(id, &logins)
}

fn password(id: &str, login: &Login) -> Result<String, String> {
    get_secret(id, login)
}

/// Fills `login` into the login form of the content page. Blocking (keyring).
fn fill(content: &Webview, id: &str, login: &Login) -> Result<(), String> {
    let url = content.url().map_err(|e| e.to_string())?;
    if url.origin().ascii_serialization() != login.origin {
        return Err(format!("the page is not on {}", login.origin));
    }
    let password = password(id, login)?;
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
        let id = app_id(&app);
        let Ok(url) = content.url() else { return };
        if let Ok([login]) = matching(&id, &url).as_deref() {
            if let Err(e) = fill(&content, &id, login) {
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
        let id = app_id(&app);
        let logins = content
            .url()
            .map_err(|e| e.to_string())
            .and_then(|url| matching(&id, &url));
        if let Ok([login]) = logins.as_deref() {
            match fill(&content, &id, login) {
                Ok(()) => return,
                Err(e) => eprintln!("weboverlay: vault: {e}"),
            }
        }
        if let Err(e) = open(&handle) {
            eprintln!("weboverlay: cannot open the vault window: {e}");
        }
    });
}

/// `open` from a worker thread (see `editor::open_later`).
pub fn open_later(handle: &AppHandle) {
    let handle = handle.clone();
    std::thread::spawn(move || {
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

/// Vault of the app of this process.
struct Current {
    id: String,
    content: Option<Webview>,
    /// Origin of the app `url`.
    home: String,
}

/// Vault id, content webview and home origin of the app of this process (one app per process). Without app
/// window (editor-only process, `--edit`), the app of the edited config: logins can be managed,
/// not filled.
fn current(webview: &Webview) -> Result<Current, String> {
    if webview.label() != VAULT_LABEL {
        return Err("not allowed".into());
    }
    let handle = webview.app_handle();
    if let Some((label, app)) = header::first_app(handle) {
        return Ok(Current {
            id: app_id(&app),
            content: handle.get_webview(&window::content_label(&label)),
            home: app.config.url.origin().ascii_serialization(),
        });
    }
    let path = handle.state::<editor::Editor>().config_path();
    let path = std::path::absolute(&path).unwrap_or(path);
    let config = Config::load(&path).map_err(|e| format!("{e:#}"))?;
    let app = config.app.ok_or("the config has no [app]")?;
    Ok(Current {
        id: icon::app_id(&path, &app.name),
        content: None,
        home: app.url.origin().ascii_serialization(),
    })
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
    /// Origin of the content page (empty when not http(s) or the app is not running).
    current: String,
    /// Origin of the app `url`, default site of a new login.
    home: String,
    logins: Vec<Login>,
}

#[tauri::command]
pub async fn vault_list(webview: Webview) -> Result<VaultView, String> {
    let Current { id, content, home } = current(&webview)?;
    let current = content
        .and_then(|content| content.url().ok())
        .and_then(|url| origin_of(url.as_str()).ok())
        .unwrap_or_default();
    blocking(move || {
        Ok(VaultView {
            current,
            home,
            logins: list(&id)?,
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
    let id = current(&webview)?.id;
    blocking(move || save(&id, login, &password, previous)).await
}

#[tauri::command]
pub async fn vault_delete(webview: Webview, login: Login) -> Result<(), String> {
    let id = current(&webview)?.id;
    blocking(move || delete(&id, &login)).await
}

#[tauri::command]
pub async fn vault_password(webview: Webview, login: Login) -> Result<String, String> {
    let id = current(&webview)?.id;
    blocking(move || password(&id, &login)).await
}

#[tauri::command]
pub async fn vault_fill(webview: Webview, login: Login) -> Result<(), String> {
    let Current { id, content, .. } = current(&webview)?;
    let content = content.ok_or("the app is not running")?;
    blocking(move || fill(&content, &id, &login)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_files_are_private() {
        let dir = std::env::temp_dir().join(format!("weboverlay-vault-{}", std::process::id()));
        let path = dir.join("sub").join("x.basic-secrets.json");
        let mut secrets = BTreeMap::new();
        secrets.insert("https://a.com|me".to_owned(), "pw".to_owned());
        write_json(&path, &secrets).unwrap();
        let read: BTreeMap<String, String> = read_json(&path).unwrap();
        assert_eq!(read, secrets);
        let missing: BTreeMap<String, String> = read_json(&dir.join("none.json")).unwrap();
        assert!(missing.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

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
