use crate::scope::Scope;
use anyhow::{Context, Result};
use directories::BaseDirs;
use nix::unistd::{Uid, User};
use std::env;
use std::path::{Path, PathBuf};

const APP_NAME: &str = "changed";
const USER_CONFIG_ENV: &str = "CHANGED_CONFIG_HOME";
const USER_STATE_ENV: &str = "CHANGED_STATE_HOME";
const SYSTEM_CONFIG_ENV: &str = "CHANGED_SYSTEM_CONFIG_HOME";
const SYSTEM_STATE_ENV: &str = "CHANGED_SYSTEM_STATE_HOME";

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub scope: Scope,
    pub config_home: PathBuf,
    pub state_home: PathBuf,
}

impl AppPaths {
    pub fn detect(scope: Scope) -> Result<Self> {
        let (config_home, state_home) = match scope {
            Scope::User => {
                let default_config_home = user_config_root()?.join(APP_NAME);
                let default_state_home = user_state_root()?.join(APP_NAME);
                let config_home =
                    env::var_os(USER_CONFIG_ENV).map_or(default_config_home, PathBuf::from);
                let state_home =
                    env::var_os(USER_STATE_ENV).map_or(default_state_home, PathBuf::from);
                (config_home, state_home)
            }
            Scope::System => {
                let config_home = env::var_os(SYSTEM_CONFIG_ENV)
                    .map_or_else(|| PathBuf::from("/etc/changed"), PathBuf::from);
                let state_home = env::var_os(SYSTEM_STATE_ENV)
                    .map_or_else(|| PathBuf::from("/var/lib/changed"), PathBuf::from);
                (config_home, state_home)
            }
        };

        Ok(Self {
            scope,
            config_home,
            state_home,
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_home.join("config.toml")
    }

    pub fn journal_file(&self) -> PathBuf {
        self.state_home.join("journal.jsonl")
    }

    pub fn daemon_state_file(&self) -> PathBuf {
        self.state_home.join("daemon-state.json")
    }
}

pub fn expand_path(raw_path: &str) -> Result<PathBuf> {
    if raw_path == "~" {
        return home_dir();
    }
    if let Some(stripped) = raw_path.strip_prefix("~/") {
        return Ok(home_dir()?.join(stripped));
    }

    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()
            .context("failed to detect current directory")?
            .join(path))
    }
}

pub fn infer_scope_for_path(path: &Path) -> Option<Scope> {
    entry_scope(path)
}

pub fn normalize_display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn home_dir() -> Result<PathBuf> {
    user_home_dir()
}

fn entry_scope(path: &Path) -> Option<Scope> {
    let normalized = normalize_display_path(path);
    let home = home_dir().ok()?;
    let home_display = normalize_display_path(&home);

    if normalized == home_display || normalized.starts_with(&format!("{home_display}/")) {
        Some(Scope::User)
    } else if path.is_absolute() {
        Some(Scope::System)
    } else {
        None
    }
}

fn user_home_dir() -> Result<PathBuf> {
    if Uid::effective().is_root() {
        if let Some(uid) = sudo_uid()?
            && let Some(user) = User::from_uid(uid).context("failed to resolve sudo user by uid")?
        {
            return Ok(user.dir);
        }

        if let Some(username) = env::var_os("SUDO_USER")
            && let Some(user) = User::from_name(&username.to_string_lossy())
                .context("failed to resolve sudo user by name")?
        {
            return Ok(user.dir);
        }
    }

    BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .context("failed to detect home directory")
}

fn user_config_root() -> Result<PathBuf> {
    if let Some(value) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(value));
    }
    Ok(user_home_dir()?.join(".config"))
}

fn user_state_root() -> Result<PathBuf> {
    if let Some(value) = env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(value));
    }
    Ok(user_home_dir()?.join(".local/state"))
}

fn sudo_uid() -> Result<Option<Uid>> {
    let Some(raw) = env::var_os("SUDO_UID") else {
        return Ok(None);
    };

    let value = raw
        .to_string_lossy()
        .parse::<u32>()
        .context("failed to parse SUDO_UID")?;
    Ok(Some(Uid::from_raw(value)))
}
