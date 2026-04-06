use crate::scope::Scope;
use anyhow::{Context, Result, anyhow};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::paths::home_dir;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceActivity {
    Active,
    Inactive,
    Unknown,
}

pub fn daemon_binary_path() -> Result<PathBuf> {
    let current = env::current_exe().context("failed to detect current executable")?;
    let file_name = current
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let candidate = if file_name == "changedd" {
        current.clone()
    } else {
        current.with_file_name("changedd")
    };

    if candidate.exists() {
        Ok(candidate)
    } else {
        Err(anyhow!(
            "failed to find changedd next to {}",
            current.display()
        ))
    }
}

pub fn render_systemd_unit(scope: Scope, daemon_path: &Path) -> String {
    let (after_target, wanted_by, scope_flag, description) = match scope {
        Scope::System => (
            "network.target",
            "multi-user.target",
            "--system",
            "changed system tuning changelog daemon",
        ),
        Scope::User => (
            "default.target",
            "default.target",
            "--user",
            "changed user tuning changelog daemon",
        ),
    };

    format!(
        "[Unit]\nDescription={description}\nAfter={after_target}\n\n[Service]\nType=simple\nExecStart={} {}\nRestart=on-failure\nRestartSec=2\nNoNewPrivileges=yes\nPrivateTmp=yes\n\n[Install]\nWantedBy={wanted_by}\n",
        daemon_path.display(),
        scope_flag
    )
}

pub fn service_unit_install_path(scope: Scope) -> Result<PathBuf> {
    match scope {
        Scope::System => Ok(PathBuf::from("/etc/systemd/system").join(systemd_unit_name())),
        Scope::User => Ok(home_dir()?
            .join(".config/systemd/user")
            .join(systemd_unit_name())),
    }
}

pub fn run_systemctl<I, S>(scope: Scope, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut command = build_systemctl_command(scope);
    for arg in args {
        command.arg(arg.as_ref());
    }

    let output = command
        .output()
        .with_context(|| format!("failed to run systemctl for {scope} scope"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();

    if output.status.success() {
        Ok(if stdout.is_empty() {
            format!("systemctl completed for {scope} scope.")
        } else {
            stdout
        })
    } else if stderr.is_empty() {
        Err(anyhow!(
            "systemctl failed for {} scope with status {}",
            scope,
            output.status
        ))
    } else {
        Err(anyhow!(stderr))
    }
}

pub fn query_service_activity(scope: Scope) -> ServiceActivity {
    let mut command = build_systemctl_command(scope);
    command.args(["is-active", "--quiet", systemd_unit_name()]);

    match command.status() {
        Ok(status) if status.success() => ServiceActivity::Active,
        Ok(status) if status.code().is_some() => ServiceActivity::Inactive,
        Ok(_) => ServiceActivity::Unknown,
        Err(_) => ServiceActivity::Unknown,
    }
}

pub fn build_systemctl_command(scope: Scope) -> Command {
    let mut command = Command::new("systemctl");
    if scope == Scope::User {
        if let Some(username) = sudo_user_name() {
            command.arg("--machine");
            command.arg(format!("{username}@.host"));
        }
        command.arg("--user");
    }
    command
}

pub fn systemd_unit_name() -> &'static str {
    "changedd.service"
}

fn sudo_user_name() -> Option<String> {
    if !nix::unistd::Uid::effective().is_root() {
        return None;
    }

    env::var("SUDO_USER").ok().filter(|value| !value.is_empty())
}
