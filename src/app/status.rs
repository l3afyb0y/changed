use crate::category::Category;
use crate::config::Config;
use crate::scope::Scope;
use crate::setup::{CpuVendor, GpuVendor, SetupProfile, ShellKind};
use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::App;
use super::paths::normalize_display_path;
use super::service::{build_systemctl_command, service_unit_install_path, systemd_unit_name};
use super::watch::{WatchPlan, WatchRootSummary};

#[derive(Clone, Debug)]
struct ScopeStatus {
    scope: Scope,
    initialized: bool,
    setup_profile_path: PathBuf,
    setup_profile_exists: bool,
    setup_profile: Option<SetupProfile>,
    config_path: PathBuf,
    config_exists: bool,
    state_path: PathBuf,
    journal_path: PathBuf,
    journal_exists: bool,
    event_count: usize,
    last_event_timestamp: Option<OffsetDateTime>,
    daemon_state_path: PathBuf,
    daemon_state_exists: bool,
    observed_path_count: Option<usize>,
    daemon_state_updated: Option<OffsetDateTime>,
    tracked_path_count: usize,
    tracked_package_count: usize,
    categories: Vec<Category>,
    watcher_roots: Vec<WatchRootSummary>,
    service: ServiceStatus,
    warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct ServiceStatus {
    load_state: Option<String>,
    active_state: Option<String>,
    sub_state: Option<String>,
    unit_file_state: Option<String>,
    main_pid: Option<u32>,
    fragment_path: Option<PathBuf>,
    query_error: Option<String>,
    local_override_path: Option<PathBuf>,
    packaged_unit_path: Option<PathBuf>,
}

pub fn render_status_report(app: &App, scopes: &[Scope]) -> Result<String> {
    let statuses = scopes
        .iter()
        .copied()
        .map(|scope| collect_scope_status(app, scope))
        .collect::<Vec<_>>();

    let mut out = String::from("# changed status\n");
    for (index, status) in statuses.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        render_scope_status(&mut out, status);
    }
    Ok(out.trim_end().to_owned())
}

fn collect_scope_status(app: &App, scope: Scope) -> ScopeStatus {
    let paths = app.paths_for_scope(scope);
    let setup_profile_path = app.shared_setup_file();
    let setup_profile_exists = setup_profile_path.exists();
    let config_path = paths.config_file();
    let journal_path = paths.journal_file();
    let daemon_state_path = paths.daemon_state_file();
    let config_exists = config_path.exists();
    let journal_exists = journal_path.exists();
    let daemon_state_exists = daemon_state_path.exists();
    let initialized = config_exists || journal_exists || daemon_state_exists;

    let (config, config_error) = match app.load_or_default(scope) {
        Ok(config) => (config, None),
        Err(err) => (Config::new(), Some(err.to_string())),
    };

    let (event_count, last_event_timestamp, journal_error) = match app.load_events(scope) {
        Ok(events) => {
            let last = events
                .iter()
                .max_by_key(|event| event.timestamp)
                .map(|event| event.timestamp);
            (events.len(), last, None)
        }
        Err(err) => (0, None, Some(err.to_string())),
    };

    let (observed_path_count, daemon_state_error) = match app.load_daemon_state(scope) {
        Ok(state) => (Some(state.observed.len()), None),
        Err(err) => (None, Some(err.to_string())),
    };
    let (setup_profile, setup_error) = match app.load_setup_profile() {
        Ok(profile) => (profile, None),
        Err(err) => (None, Some(err.to_string())),
    };

    let daemon_state_updated = file_timestamp(&daemon_state_path);
    let categories = collect_categories(&config);
    let watcher_roots = WatchPlan::new(&config, paths).root_summaries();
    let service = query_service_status(scope);

    let mut warnings = Vec::new();
    if let Some(error) = &config_error {
        warnings.push(format!("Config could not be loaded: {error}"));
    }
    if let Some(error) = &journal_error {
        warnings.push(format!("Journal could not be read: {error}"));
    }
    if let Some(error) = &daemon_state_error {
        warnings.push(format!("Daemon state could not be read: {error}"));
    }
    if let Some(error) = &setup_error {
        warnings.push(format!("Setup profile could not be read: {error}"));
    }
    if !config_exists && !journal_exists && !daemon_state_exists {
        warnings.push(String::from("Scope is not initialized yet."));
    }
    if !setup_profile_exists {
        warnings.push(String::from(
            "Machine setup has not been run yet. Some preset-backed paths may still be missing.",
        ));
    }
    if config_exists && config.tracked_paths.is_empty() && config.tracked_packages.is_empty() {
        warnings.push(String::from("Nothing is currently tracked for this scope."));
    }
    if service.is_active() && config.tracked_paths.is_empty() && config.tracked_packages.is_empty()
    {
        warnings.push(String::from(
            "Service is running, but nothing is tracked in this scope.",
        ));
    }
    if !service.is_active() && (config_exists || journal_exists || daemon_state_exists) {
        warnings.push(String::from(
            "Service is not active for this scope. History will not update until the daemon is running.",
        ));
    }
    if service.local_override_path.is_some() && service.packaged_unit_path.is_some() {
        warnings.push(format!(
            "A local unit override is present at {} and may shadow the packaged unit.",
            service
                .local_override_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        ));
    }
    if let Some(error) = &service.query_error {
        warnings.push(format!("Could not query systemd service state: {error}"));
    }

    ScopeStatus {
        scope,
        initialized,
        setup_profile_path,
        setup_profile_exists,
        setup_profile,
        config_path,
        config_exists,
        state_path: paths.state_home.clone(),
        journal_path,
        journal_exists,
        event_count,
        last_event_timestamp,
        daemon_state_path,
        daemon_state_exists,
        observed_path_count,
        daemon_state_updated,
        tracked_path_count: config.tracked_paths.len(),
        tracked_package_count: config.tracked_packages.len(),
        categories,
        watcher_roots,
        service,
        warnings,
    }
}

fn collect_categories(config: &Config) -> Vec<Category> {
    let mut categories = BTreeSet::new();
    for entry in &config.tracked_paths {
        categories.insert(entry.category);
    }
    if !config.tracked_packages.is_empty() {
        categories.insert(Category::Packages);
    }
    categories.into_iter().collect()
}

fn query_service_status(scope: Scope) -> ServiceStatus {
    let mut service = ServiceStatus {
        local_override_path: service_unit_install_path(scope)
            .ok()
            .filter(|path| path.exists()),
        packaged_unit_path: packaged_unit_path(scope).filter(|path| path.exists()),
        ..ServiceStatus::default()
    };

    let mut command = build_systemctl_command(scope);
    command.args([
        "show",
        systemd_unit_name(),
        "--property=LoadState",
        "--property=ActiveState",
        "--property=SubState",
        "--property=UnitFileState",
        "--property=MainPID",
        "--property=FragmentPath",
    ]);

    match command.output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };
                if value.is_empty() {
                    continue;
                }
                match key {
                    "LoadState" => service.load_state = Some(value.to_owned()),
                    "ActiveState" => service.active_state = Some(value.to_owned()),
                    "SubState" => service.sub_state = Some(value.to_owned()),
                    "UnitFileState" => service.unit_file_state = Some(value.to_owned()),
                    "MainPID" => {
                        service.main_pid = value.parse::<u32>().ok().filter(|pid| *pid > 0)
                    }
                    "FragmentPath" => service.fragment_path = Some(PathBuf::from(value)),
                    _ => {}
                }
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            service.query_error = Some(if stderr.is_empty() {
                format!("systemctl exited with status {}", output.status)
            } else {
                stderr
            });
        }
        Err(err) => service.query_error = Some(err.to_string()),
    }

    service
}

fn packaged_unit_path(scope: Scope) -> Option<PathBuf> {
    let base = match scope {
        Scope::System => PathBuf::from("/usr/lib/systemd/system"),
        Scope::User => PathBuf::from("/usr/lib/systemd/user"),
    };
    Some(base.join(systemd_unit_name()))
}

fn file_timestamp(path: &Path) -> Option<OffsetDateTime> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(OffsetDateTime::from(modified))
}

fn format_timestamp(value: Option<OffsetDateTime>) -> String {
    value
        .and_then(|timestamp| timestamp.format(&Rfc3339).ok())
        .unwrap_or_else(|| String::from("none"))
}

fn render_scope_status(out: &mut String, status: &ScopeStatus) {
    let _ = std::fmt::Write::write_fmt(out, format_args!("\n## {}\n", status.scope));
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(
            "Initialized: {}\n",
            if status.initialized { "yes" } else { "no" }
        ),
    );
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(
            "Setup profile: {}{}\n",
            status.setup_profile_path.display(),
            existence_label(status.setup_profile_exists)
        ),
    );
    if let Some(profile) = &status.setup_profile {
        let cpu = profile
            .cpu_vendor
            .map(cpu_vendor_label)
            .unwrap_or("unknown");
        let gpus = if profile.gpu_vendors.is_empty() {
            String::from("none")
        } else {
            profile
                .gpu_vendors
                .iter()
                .map(|vendor| gpu_vendor_label(*vendor))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let shells = if profile.shells.is_empty() {
            String::from("none")
        } else {
            profile
                .shells
                .iter()
                .map(|shell| shell_kind_label(*shell))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = std::fmt::Write::write_fmt(
            out,
            format_args!("Setup details: cpu={cpu} | gpu={gpus} | shells={shells}\n"),
        );
    }
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(
            "Config: {}{}\n",
            status.config_path.display(),
            existence_label(status.config_exists)
        ),
    );
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!("State: {}\n", status.state_path.display()),
    );
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(
            "Journal: {}{} | events: {} | last event: {}\n",
            status.journal_path.display(),
            existence_label(status.journal_exists),
            status.event_count,
            format_timestamp(status.last_event_timestamp)
        ),
    );
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(
            "Daemon state: {}{} | observed paths: {} | last update: {}\n",
            status.daemon_state_path.display(),
            existence_label(status.daemon_state_exists),
            status
                .observed_path_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| String::from("unknown")),
            format_timestamp(status.daemon_state_updated)
        ),
    );
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(
            "Tracked: {} path{} | {} package{}\n",
            status.tracked_path_count,
            pluralize(status.tracked_path_count),
            status.tracked_package_count,
            pluralize(status.tracked_package_count)
        ),
    );
    let categories = if status.categories.is_empty() {
        String::from("none")
    } else {
        status
            .categories
            .iter()
            .map(|category| category.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let _ = std::fmt::Write::write_fmt(out, format_args!("Categories: {categories}\n"));
    render_service_status(out, &status.service);
    render_watcher_roots(out, &status.watcher_roots);

    if status.warnings.is_empty() {
        out.push_str("Warnings: none\n");
    } else {
        out.push_str("Warnings:\n");
        for warning in &status.warnings {
            let _ = std::fmt::Write::write_fmt(out, format_args!("  - {warning}\n"));
        }
    }
}

fn render_service_status(out: &mut String, service: &ServiceStatus) {
    let active_state = service.active_state.as_deref().unwrap_or("unknown");
    let sub_state = service.sub_state.as_deref().unwrap_or("unknown");
    let unit_file_state = service.unit_file_state.as_deref().unwrap_or("unknown");
    let pid = service
        .main_pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| String::from("none"));
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(
            "Service: {} ({}) | enabled: {} | pid: {}\n",
            active_state, sub_state, unit_file_state, pid
        ),
    );
    if let Some(fragment) = &service.fragment_path {
        let _ =
            std::fmt::Write::write_fmt(out, format_args!("Service unit: {}\n", fragment.display()));
    }
}

fn render_watcher_roots(out: &mut String, roots: &[WatchRootSummary]) {
    if roots.is_empty() {
        out.push_str("Watcher roots: none\n");
        return;
    }

    out.push_str("Watcher roots:\n");
    for root in roots {
        let mode = if root.recursive {
            "recursive"
        } else {
            "non-recursive"
        };
        let _ = std::fmt::Write::write_fmt(
            out,
            format_args!("  - {} ({mode})\n", normalize_display_path(&root.path)),
        );
    }
}

fn existence_label(exists: bool) -> &'static str {
    if exists { " [present]" } else { " [missing]" }
}

fn pluralize(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn cpu_vendor_label(vendor: CpuVendor) -> &'static str {
    match vendor {
        CpuVendor::Intel => "intel",
        CpuVendor::Amd => "amd",
    }
}

fn gpu_vendor_label(vendor: GpuVendor) -> &'static str {
    match vendor {
        GpuVendor::Nvidia => "nvidia",
        GpuVendor::Amd => "amd",
        GpuVendor::Intel => "intel",
    }
}

fn shell_kind_label(shell: ShellKind) -> &'static str {
    match shell {
        ShellKind::Bash => "bash",
        ShellKind::Fish => "fish",
        ShellKind::Zsh => "zsh",
    }
}

impl ServiceStatus {
    fn is_active(&self) -> bool {
        self.active_state.as_deref() == Some("active")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_render_mentions_missing_initialization_and_service_inactive() {
        let status = ScopeStatus {
            scope: Scope::User,
            initialized: false,
            setup_profile_path: PathBuf::from("/tmp/setup.toml"),
            setup_profile_exists: false,
            setup_profile: None,
            config_path: PathBuf::from("/tmp/config.toml"),
            config_exists: false,
            state_path: PathBuf::from("/tmp/state"),
            journal_path: PathBuf::from("/tmp/state/journal.jsonl"),
            journal_exists: false,
            event_count: 0,
            last_event_timestamp: None,
            daemon_state_path: PathBuf::from("/tmp/state/daemon-state.json"),
            daemon_state_exists: false,
            observed_path_count: Some(0),
            daemon_state_updated: None,
            tracked_path_count: 0,
            tracked_package_count: 0,
            categories: Vec::new(),
            watcher_roots: Vec::new(),
            service: ServiceStatus::default(),
            warnings: vec![
                String::from("Scope is not initialized yet."),
                String::from(
                    "Service is not active for this scope. History will not update until the daemon is running.",
                ),
            ],
        };

        let mut rendered = String::new();
        render_scope_status(&mut rendered, &status);

        assert!(rendered.contains("Initialized: no"));
        assert!(rendered.contains("Warnings:"));
        assert!(rendered.contains("Scope is not initialized yet."));
    }

    #[test]
    fn service_status_detects_active_service() {
        let service = ServiceStatus {
            active_state: Some(String::from("active")),
            ..ServiceStatus::default()
        };

        assert!(service.is_active());
    }
}
