#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod editor;
mod header;
mod icon;
mod inject;
mod launcher;
mod schema;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::window::AppWindow;

const USAGE: &str = "\
Usage: weboverlay [--config <path>] [--list] [--edit] [--maximized] [<app>...]

Opens the given apps from the config (all apps if none given). Without app nor --config,
opens the launcher (recent apps and the apps of the local configs).

Options:
  -c, --config <path>  Config file (default: config.toml next to the executable,
                       else <config dir>/weboverlay/config.toml)
  -l, --list           List configured apps and exit
  -e, --edit           Open the config editor only (works with a missing or invalid config)
  -m, --maximized      Start the windows maximized (overrides the config)
  -h, --help           Show this help";

struct Args {
    config: Option<PathBuf>,
    list: bool,
    edit: bool,
    maximized: bool,
    apps: Vec<String>,
}

fn parse_args() -> Result<Option<Args>> {
    let mut args = Args {
        config: None,
        list: false,
        edit: false,
        maximized: false,
        apps: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
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
            flag if flag.starts_with('-') => bail!("unknown option {flag}\n\n{USAGE}"),
            name => args.apps.push(name.to_owned()),
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
    /// Empty unless `Mode::Apps`.
    apps: Vec<Arc<AppWindow>>,
    mode: Mode,
}

fn prepare() -> Result<Option<Launch>> {
    let Some(args) = parse_args()? else {
        return Ok(None);
    };
    let launcher = args.config.is_none() && args.apps.is_empty() && !args.edit && !args.list;
    let config_path = match args.config {
        Some(path) => path,
        None => config::default_path()?,
    };
    if launcher {
        return Ok(Some(Launch {
            config_path,
            apps: Vec::new(),
            mode: Mode::Launcher,
        }));
    }
    if args.edit {
        // The config may be missing or invalid: that's what the editor is for.
        return Ok(Some(Launch {
            config_path,
            apps: Vec::new(),
            mode: Mode::Edit,
        }));
    }
    let mut config = Config::load(&config_path)?;

    if args.list {
        for app in &config.apps {
            println!("{}\t{}", app.name, app.url);
        }
        return Ok(None);
    }

    for name in &args.apps {
        if config.find(name).is_none() {
            bail!("unknown app {name:?} in {}", config_path.display());
        }
    }
    if !args.apps.is_empty() {
        config.apps.retain(|app| args.apps.contains(&app.name));
    }
    if config.apps.is_empty() {
        bail!("no app defined in {}", config_path.display());
    }

    // Not `canonicalize`: it yields `\\?\` paths on Windows.
    let absolute_config = std::path::absolute(&config_path).unwrap_or_else(|_| config_path.clone());
    let profiles = dirs::data_local_dir()
        .context("cannot determine the user data directory")?
        .join("weboverlay")
        .join("profiles");
    let apps = config
        .apps
        .into_iter()
        .map(|mut app| {
            app.maximized |= args.maximized;
            let scripts = inject::build_scripts(&app)?;
            let profile_dir = profiles.join(&app.name);
            Ok(Arc::new(AppWindow {
                config: app,
                scripts,
                profile_dir,
                config_path: absolute_config.clone(),
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    launcher::record(&config_path, &apps);
    Ok(Some(Launch {
        config_path,
        apps,
        mode: Mode::Apps,
    }))
}

fn main() -> ExitCode {
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
            config_path: launch.config_path,
            relaunch_args: editor::strip_edit_flag(std::env::args_os().skip(1)),
        })
        .invoke_handler(tauri::generate_handler![
            header::header_action,
            editor::editor_files,
            editor::editor_read,
            editor::editor_write,
            editor::editor_restart,
            editor::editor_schema,
            editor::editor_check,
            launcher::launcher_list,
            launcher::launcher_icon,
            launcher::launcher_open,
            launcher::launcher_edit,
            launcher::launcher_create,
            launcher::launcher_forget_recent,
            launcher::launcher_forget,
            launcher::launcher_window,
        ])
        .setup(move |app| {
            match launch.mode {
                Mode::Apps => {}
                Mode::Edit => editor::open(app.handle())?,
                Mode::Launcher => launcher::open(app.handle())?,
            }
            for window in launch.apps {
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
