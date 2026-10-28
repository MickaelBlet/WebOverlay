#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod header;
mod icon;
mod inject;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::window::AppWindow;

const USAGE: &str = "\
Usage: weboverlay [--config <path>] [--list] [--maximized] [<app>...]

Opens the given apps from the config (all apps if none given).

Options:
  -c, --config <path>  Config file (default: config.toml next to the executable,
                       else <config dir>/weboverlay/config.toml)
  -l, --list           List configured apps and exit
  -m, --maximized      Start the windows maximized (overrides the config)
  -h, --help           Show this help";

struct Args {
    config: Option<PathBuf>,
    list: bool,
    maximized: bool,
    apps: Vec<String>,
}

fn parse_args() -> Result<Option<Args>> {
    let mut args = Args {
        config: None,
        list: false,
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

fn prepare() -> Result<Option<Vec<Arc<AppWindow>>>> {
    let Some(args) = parse_args()? else {
        return Ok(None);
    };
    let config_path = match args.config {
        Some(path) => path,
        None => config::default_path()?,
    };
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

    let profiles = dirs::data_local_dir()
        .context("cannot determine the user data directory")?
        .join("weboverlay")
        .join("profiles");
    config
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
            }))
        })
        .collect::<Result<_>>()
        .map(Some)
}

fn main() -> ExitCode {
    let apps = match prepare() {
        Ok(Some(apps)) => apps,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("weboverlay: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    let result = tauri::Builder::default()
        .manage(header::States::default())
        .invoke_handler(tauri::generate_handler![header::header_action])
        .setup(move |app| {
            for window in apps {
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
