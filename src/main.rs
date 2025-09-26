use clap::Parser;
use nanoid::nanoid;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

use anyhow::{Result, anyhow, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
const APP_NAME: &str = env!("CARGO_PKG_NAME");
const WINEPREFIX: &str = "WINEPREFIX";

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    data_dir: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug)]
struct PathMap {
    #[serde(default)]
    path_map: HashMap<PathBuf, String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ExecEnv {
    #[serde(default)]
    executed_tricks: HashSet<String>,
}

#[derive(Parser, Debug)]
#[command(version,about,long_about = None)]
struct Args {
    /// Run winetricks commands when it is not yet executed.
    #[arg(long)]
    with_tricks: Vec<String>,
    /// Path to exe file.
    exec_path: PathBuf,
    /// Arguments for exe.
    args: Vec<String>,
}

fn main() -> Result<()> {
    run(Args::parse())
}

fn run(args: Args) -> Result<()> {
    let data_dir = prepare()?;
    let exec_env_path =
        if let Some(exec_env_path) = get_base_env_dir_from_exec_path(&args.exec_path, &data_dir) {
            exec_env_path
        } else {
            get_env_dir(&args.exec_path, &data_dir)?
        };
    if !exec_env_path.exists() {
        fs::create_dir_all(&exec_env_path)?;
    }

    let exec_env_conf_path = exec_env_path.join("conf.toml");
    let mut exec_env_conf_buf = vec![];
    {
        let mut exec_env_conf_file = if exec_env_conf_path.exists() {
            File::open(&exec_env_conf_path)?
        } else {
            File::create_new(&exec_env_conf_path)?
        };
        exec_env_conf_file.read_to_end(&mut exec_env_conf_buf)?;
    }
    let mut exec_conf = toml::from_slice::<ExecEnv>(&exec_env_conf_buf)?;
    let exec_env_wine_path = exec_env_path.join(".wine");
    if !exec_env_wine_path.exists() {
        fs::create_dir_all(&exec_env_wine_path)?;
    }

    println!("Resolve winetricks...");
    for trick in args.with_tricks {
        for trick in trick.split(",") {
            if !exec_conf.executed_tricks.contains(trick) {
                exec_conf.executed_tricks.insert(trick.to_string());
                let status = exec_command("winetricks", &[trick.to_string()], &exec_env_wine_path)?;
                if !status.success() {
                    bail!("winetricks is not succeed {trick}, status:{status}");
                }
                fs::write(
                    &exec_env_conf_path,
                    toml::to_string_pretty(&exec_conf)?.as_bytes(),
                )?;
            }
        }
    }
    let exec_path_str = args.exec_path.to_string_lossy().to_string();
    let mut wine_args = vec![exec_path_str.clone()];
    wine_args.extend_from_slice(&args.args);
    println!("Run wine {exec_path_str}");
    let status = exec_command("wine", wine_args, exec_env_wine_path)?;
    if !status.success() {
        bail!("wine is not succeed {status}");
    }

    Ok(())
}
fn exec_command<I, S>(
    command: impl AsRef<str>,
    args: I,
    wine_prefix: impl AsRef<Path>,
) -> Result<ExitStatus>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Ok(Command::new(command.as_ref())
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env(WINEPREFIX, wine_prefix.as_ref().as_os_str())
        .status()?)
}
fn get_env_dir(exec_path: impl AsRef<Path>, data_dir: impl AsRef<Path>) -> Result<PathBuf> {
    let exec_path = exec_path.as_ref();
    let data_dir = data_dir.as_ref();
    let path_map_path = data_dir.join("path_map.toml");
    let mut path_map_data = vec![];
    {
        let mut path_map_file = if path_map_path.exists() {
            File::open(&path_map_path)?
        } else {
            File::create_new(&path_map_path)?
        };
        path_map_file.read_to_end(&mut path_map_data)?;
    }
    let mut path_map = toml::from_slice::<PathMap>(&path_map_data)?;
    let id = if let Some(id) = path_map.path_map.get(exec_path) {
        id.clone()
    } else {
        let id = nanoid!();
        path_map
            .path_map
            .insert(exec_path.to_path_buf(), id.clone());
        fs::write(
            path_map_path.as_path(),
            toml::to_string_pretty(&path_map)?.as_bytes(),
        )?;
        id
    };
    Ok(data_dir.join(id))
}

fn get_base_env_dir_from_exec_path(
    exec_path: impl AsRef<Path>,
    data_dir: impl AsRef<Path>,
) -> Option<PathBuf> {
    let exec_path = exec_path.as_ref();
    let data_dir = data_dir.as_ref();
    let mut base_wine_prefix_dir = None;
    let mut taget_path = exec_path;
    while let Some(parent_dir) = taget_path.parent() {
        let name = parent_dir
            .file_name()
            .map(|n| n.to_str().unwrap_or(""))
            .unwrap_or("");
        if name == ".wine" {
            base_wine_prefix_dir = Some(parent_dir);
            break;
        }
        taget_path = parent_dir;
    }
    if let Some(base_wine_prefix_dir) = base_wine_prefix_dir
        && base_wine_prefix_dir
            .to_string_lossy()
            .contains(data_dir.to_string_lossy().as_ref())
        && let Some(base_env_dir) = base_wine_prefix_dir.parent()
        && base_env_dir.join("conf.toml").exists()
    {
        Some(base_env_dir.to_path_buf())
    } else {
        None
    }
}
fn prepare() -> Result<PathBuf> {
    if let Some(project_dirs) = ProjectDirs::from("", "", APP_NAME) {
        if !project_dirs.data_dir().exists() {
            fs::create_dir_all(project_dirs.data_dir())?;
        }
        let conf_path = project_dirs.data_dir().join("config.toml");
        let mut conf_data = vec![];
        {
            let mut conf_file = if !conf_path.exists() {
                File::create_new(&conf_path)?
            } else {
                File::open(&conf_path)?
            };
            conf_file.read_to_end(&mut conf_data)?;
        }
        let mut conf = toml::from_slice::<Config>(&conf_data)?;
        if conf.data_dir.is_none() {
            conf.data_dir = Some(project_dirs.data_local_dir().to_path_buf());
            let save_data = toml::to_string_pretty(&conf)?;
            fs::write(&conf_path, save_data.as_bytes())?;
        }
        let data_dir = conf.data_dir.unwrap();
        if !data_dir.exists() {
            fs::create_dir_all(&data_dir)?;
        }
        Ok(data_dir)
    } else {
        Err(anyhow!("Can not create project dir."))
    }
}
