#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod download;
mod editor;
mod header;
mod icon;
mod inject;
mod launcher;
#[cfg(target_os = "linux")]
mod linux;
mod schema;
mod vault;
mod window;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::window::AppWindow;

const USAGE: &str = "\
Usage: weboverlay [--config <path>] [--list] [--edit] [--maximized] [--password-store <store>]

Opens the app of the config (one app per config file). Without --config, opens the launcher
(recent apps and the apps of the local configs).

Options:
  -c, --config <path>  Config file (default: config.toml next to the executable,
                       else <config dir>/weboverlay/config.toml)
  -l, --list           Print the app of the config and exit
  -e, --edit           Open the config editor only (works with a missing or invalid config)
  -m, --maximized      Start the windows maximized (overrides the config)
  --password-store <store>
                       Linux: where the vault keeps passwords: keyring (Secret Service,
                       default) or basic (plaintext file readable by your user only)
  -h, --help           Show this help";

struct Args {
    config: Option<PathBuf>,
    list: bool,
    edit: bool,
    maximized: bool,
    /// `--password-store basic`.
    basic_store: bool,
}

/// Value of `--password-store`: `true` for `basic`.
fn password_store(value: &str) -> Result<bool> {
    if !cfg!(target_os = "linux") {
        bail!("--password-store is Linux only");
    }
    match value {
        "basic" => Ok(true),
        "keyring" => Ok(false),
        other => bail!("unknown password store {other:?} (keyring or basic)"),
    }
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut args = Args {
        config: None,
        list: false,
        edit: false,
        maximized: false,
        basic_store: false,
    };
    while let Some(arg) = it.next() {
        if let Some(value) = arg.strip_prefix("--password-store=") {
            args.basic_store = password_store(value)?;
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-l" | "--list" => args.list = true,
            "-e" | "--edit" => args.edit = true,
            "-m" | "--maximized" => args.maximized = true,
            "-c" | "--config" => {
                args.config = Some(it.next().context("--config needs a path")?.into());
            }
            "--password-store" => {
                let value = it.next().context("--password-store needs a value")?;
                args.basic_store = password_store(&value)?;
            }
            flag if flag.starts_with('-') => bail!("unknown option {flag}\n\n{USAGE}"),
            other => bail!("unexpected argument {other:?} (one app per config)\n\n{USAGE}"),
        }
    }
    Ok(Some(args))
}

enum Mode {
    Apps,
    Edit,
    Launcher,
}

struct Launch {
    config_path: PathBuf,
    /// `None` unless `Mode::Apps`.
    app: Option<Arc<AppWindow>>,
    mode: Mode,
    /// `--maximized`, also applied by the editor's Restart.
    maximized: bool,
}

fn prepare() -> Result<Option<Launch>> {
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        return Ok(None);
    };
    vault::set_basic(args.basic_store);
    let launcher = args.config.is_none() && !args.edit && !args.list;
    let config_path = match args.config {
        Some(path) => path,
        None => config::default_path()?,
    };
    if launcher {
        return Ok(Some(Launch {
            config_path,
            app: None,
            mode: Mode::Launcher,
            maximized: args.maximized,
        }));
    }
    if args.edit {
        // The config may be missing or invalid: that's what the editor is for.
        return Ok(Some(Launch {
            config_path,
            app: None,
            mode: Mode::Edit,
            maximized: args.maximized,
        }));
    }
    if args.list {
        if let Some(app) = Config::load(&config_path)?.app {
            println!("{}\t{}", app.name, app.url);
        }
        return Ok(None);
    }
    let app = load_app(&config_path, args.maximized)?;
    launcher::record(&config_path, &app.config);
    Ok(Some(Launch {
        config_path,
        app: Some(app),
        mode: Mode::Apps,
        maximized: args.maximized,
    }))
}

/// Loads the config and builds the window of its app.
pub fn load_app(config_path: &Path, maximized: bool) -> Result<Arc<AppWindow>> {
    let Some(mut app) = Config::load(config_path)?.app else {
        bail!("no [app] defined in {}", config_path.display());
    };

    // Not `canonicalize`: it yields `\\?\` paths on Windows.
    let absolute_config =
        std::path::absolute(config_path).unwrap_or_else(|_| config_path.to_owned());
    let profiles = dirs::data_local_dir()
        .context("cannot determine the user data directory")?
        .join("weboverlay")
        .join("profiles");
    app.maximized |= maximized;
    let scripts = inject::build_scripts(&app)?;
    let icon = icon::load_configured(&app)?;
    let profile_dir = profiles.join(&app.name);
    Ok(Arc::new(AppWindow {
        config: app,
        scripts,
        icon,
        profile_dir,
        config_path: absolute_config,
    }))
}

fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    linux::setup_env();
    let launch = match prepare() {
        Ok(Some(launch)) => launch,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("weboverlay: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    let result = tauri::Builder::default()
        .manage(header::States::default())
        .manage(editor::Editor {
            config_path: std::sync::Mutex::new(launch.config_path),
            maximized: launch.maximized,
            dirty: Default::default(),
            import: Default::default(),
        })
        .invoke_handler(tauri::generate_handler![
            header::header_action,
            editor::editor_files,
            editor::editor_read,
            editor::editor_write,
            editor::editor_restart,
            editor::editor_schema,
            editor::editor_check,
            editor::editor_pin,
            editor::editor_vault,
            editor::editor_window,
            editor::editor_dirty,
            editor::editor_add,
            editor::editor_pick_import,
            editor::editor_unreference,
            editor::editor_rename,
            editor::editor_rename_config,
            editor::editor_delete,
            editor::editor_reveal,
            launcher::launcher_list,
            launcher::launcher_icon,
            launcher::launcher_open,
            launcher::launcher_edit,
            launcher::launcher_create,
            launcher::launcher_import,
            launcher::launcher_forget_recent,
            launcher::launcher_forget,
            launcher::launcher_window,
            download::downloads_list,
            download::downloads_action,
            vault::vault_list,
            vault::vault_save,
            vault::vault_delete,
            vault::vault_password,
            vault::vault_fill,
        ])
        .setup(move |app| {
            match launch.mode {
                Mode::Apps => {}
                Mode::Edit => editor::open(app.handle())?,
                Mode::Launcher => launcher::open(app.handle())?,
            }
            if let Some(window) = launch.app {
                window::open(app.handle(), window)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("weboverlay: {e:#}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Args>> {
        parse_args(args.iter().map(|a| a.to_string()))
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn password_store_flag() {
        assert!(!parse(&[]).unwrap().unwrap().basic_store);
        assert!(
            parse(&["--password-store=basic"])
                .unwrap()
                .unwrap()
                .basic_store
        );
        assert!(
            parse(&["--password-store", "basic"])
                .unwrap()
                .unwrap()
                .basic_store
        );
        assert!(
            !parse(&["--password-store=keyring"])
                .unwrap()
                .unwrap()
                .basic_store
        );
        assert!(parse(&["--password-store=gnome"]).is_err());
        assert!(parse(&["--password-store"]).is_err());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn password_store_flag_linux_only() {
        assert!(parse(&["--password-store=basic"]).is_err());
    }
}
