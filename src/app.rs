mod daemon;
mod paths;
mod render;
mod service;
mod setup;
mod status;
mod watch;

use crate::category::Category;
use crate::config::{
    Config, DiffMode, PathKind, RedactionMode, RetentionPolicy, TrackSource, TrackedPackage,
    TrackedPath,
};
use crate::journal::JournalEvent;
use crate::scope::Scope;
use crate::setup::SetupProfile;
use anyhow::{Context, Result, anyhow};
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub use paths::AppPaths;

#[derive(Clone, Debug)]
pub struct App {
    pub user_paths: AppPaths,
    pub system_paths: AppPaths,
}

#[derive(Clone, Debug)]
pub struct DaemonOptions {
    pub once: bool,
}

pub struct HistoryQuery<'a> {
    pub scopes: &'a [Scope],
    pub include: &'a [Category],
    pub exclude: &'a [Category],
    pub path: Option<&'a Path>,
    pub all: bool,
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub clean: bool,
    pub color: bool,
}

impl App {
    pub fn new() -> Result<Self> {
        Ok(Self {
            user_paths: AppPaths::detect(Scope::User)?,
            system_paths: AppPaths::detect(Scope::System)?,
        })
    }

    pub(crate) fn paths_for_scope(&self, scope: Scope) -> &AppPaths {
        match scope {
            Scope::System => &self.system_paths,
            Scope::User => &self.user_paths,
        }
    }

    pub fn init(&self, scope: Scope) -> Result<String> {
        let paths = self.paths_for_scope(scope);
        ensure_scope_directories(paths)?;

        let config_path = paths.config_file();
        if config_path.exists() {
            let config = self.load_config(scope)?;
            return Ok(render::render_init_summary(paths, &config, false));
        }

        let mut config = Config::new();
        let include_setup_only = self.load_setup_profile()?.is_some();
        config.tracked_paths = detect_presets(self, scope, include_setup_only)?;
        config.sort_and_dedup();
        self.save_config(scope, &config)?;

        Ok(render::render_init_summary(paths, &config, true))
    }

    pub fn list_tracked(
        &self,
        scopes: &[Scope],
        include: &[Category],
        exclude: &[Category],
        path: Option<&Path>,
        color: bool,
    ) -> Result<String> {
        let mut scoped_configs = Vec::new();
        let path_filter = path.map(paths::normalize_display_path);
        let filters = render::CategoryFilters::new(include, exclude);

        for &scope in scopes {
            let config = self.load_or_default(scope)?;
            if config.tracked_paths.is_empty() && config.tracked_packages.is_empty() {
                continue;
            }
            scoped_configs.push((scope, config));
        }

        Ok(render::render_tracked(
            &scoped_configs,
            filters,
            path_filter.as_deref(),
            color,
        ))
    }

    pub fn list_history(&self, query: HistoryQuery<'_>) -> Result<String> {
        let filters = render::CategoryFilters::new(query.include, query.exclude);
        let since = parse_filter_time(query.since)?;
        let until = parse_filter_time(query.until)?;
        let limit = if query.all { None } else { Some(50) };
        let path_filter = query.path.map(paths::normalize_display_path);

        let mut events = Vec::new();
        let mut any_journal = false;
        for &scope in query.scopes {
            let paths = self.paths_for_scope(scope);
            any_journal |= paths.journal_file().exists();
            events.extend(self.load_filtered_events(
                scope,
                filters,
                path_filter.as_deref(),
                since,
                until,
                None,
            )?);
        }

        events.sort_by_key(|event| event.timestamp);
        if let Some(max) = limit
            && events.len() > max
        {
            events = events.split_off(events.len() - max);
        }

        if events.is_empty() {
            return Ok(if any_journal {
                String::from("No change history matched that filter.")
            } else {
                String::from("No change history recorded yet.")
            });
        }
        Ok(render::render_history(
            &events,
            query.clean,
            None,
            query.color,
        ))
    }

    pub fn run_daemon(&self, scope: Scope, options: DaemonOptions) -> Result<String> {
        daemon::run(self, scope, options)
    }

    pub fn status_report(&self, scopes: &[Scope]) -> Result<String> {
        status::render_status_report(self, scopes)
    }

    pub fn setup(&self) -> Result<String> {
        self.setup_with_profile(setup::detect_setup_profile())
    }

    pub(crate) fn setup_with_profile(&self, mut profile: SetupProfile) -> Result<String> {
        profile.sort_and_dedup();
        self.save_setup_profile(&profile)?;
        let user_paths = self.apply_setup_presets_to_scope(Scope::User, true)?;
        let system_paths = self.apply_setup_presets_to_scope(Scope::System, true)?;

        Ok(render_setup_summary(
            &self.shared_setup_file(),
            &system_paths,
            &user_paths,
        ))
    }

    pub fn track_file(&self, scope: Scope, raw_path: &str) -> Result<String> {
        let mut config = self.load_or_default(scope)?;
        let expanded = paths::expand_path(raw_path)?;
        if !expanded.exists() {
            return Err(anyhow!(
                "File not found: {}",
                paths::normalize_display_path(&expanded)
            ));
        }
        let kind = detect_path_kind(&expanded);
        let path = paths::normalize_display_path(&expanded);
        let category = infer_category_for_path(&expanded);
        let diff_mode = default_diff_mode_for_category(category);
        let redaction = default_redaction_for_category(category);

        upsert_path(
            &mut config,
            TrackedPath {
                path: path.clone(),
                category,
                kind,
                diff_mode,
                redaction,
                source: TrackSource::Manual,
            },
        );
        self.save_config(scope, &config)?;

        Ok(format!(
            "Now tracking {} in {} scope under '{}' ({}, redaction {}).",
            path,
            scope,
            category,
            diff_mode_label(diff_mode),
            redaction_label(redaction)
        ))
    }

    pub fn track_category(&self, scope: Scope, category: Category) -> Result<String> {
        let mut config = self.load_or_default(scope)?;
        let preset_targets = preset_targets_for_category(self, scope, category)?;
        if preset_targets.is_empty() {
            return Ok(format!(
                "No matching preset paths were found for '{}' in {} scope.",
                category, scope
            ));
        }

        let count_before = config.tracked_paths.len();
        for target in preset_targets {
            upsert_path(&mut config, target);
        }
        self.save_config(scope, &config)?;

        let added = config.tracked_paths.len().saturating_sub(count_before);
        Ok(format!(
            "Tracked category '{}' in {} scope. Added {} preset target{}.",
            category,
            scope,
            added,
            pluralize(added)
        ))
    }

    pub fn track_package(&self, scope: Scope, manager: &str, package_name: &str) -> Result<String> {
        let mut config = self.load_or_default(scope)?;
        config.tracked_packages.push(TrackedPackage {
            manager: manager.to_owned(),
            package_name: package_name.to_owned(),
            source: TrackSource::Manual,
        });
        config.sort_and_dedup();
        self.save_config(scope, &config)?;

        Ok(format!(
            "Now tracking package target in {} scope: {} {}",
            scope, manager, package_name
        ))
    }

    pub fn untrack_file(&self, scope: Scope, raw_path: &str) -> Result<String> {
        let mut config = self.load_or_default(scope)?;
        let path = paths::normalize_display_path(&paths::expand_path(raw_path)?);
        let before = config.tracked_paths.len();
        config.tracked_paths.retain(|entry| entry.path != path);
        self.save_config(scope, &config)?;

        let removed = before.saturating_sub(config.tracked_paths.len());
        Ok(format!(
            "Stopped tracking {} in {} scope. Removed {} target{}.",
            path,
            scope,
            removed,
            pluralize(removed)
        ))
    }

    pub fn untrack_category(&self, scope: Scope, category: Category) -> Result<String> {
        let mut config = self.load_or_default(scope)?;
        let before = config.tracked_paths.len();
        config
            .tracked_paths
            .retain(|entry| entry.category != category);
        self.save_config(scope, &config)?;

        let removed = before.saturating_sub(config.tracked_paths.len());
        Ok(format!(
            "Stopped tracking category '{}' in {} scope. Removed {} target{}.",
            category,
            scope,
            removed,
            pluralize(removed)
        ))
    }

    pub fn untrack_package(
        &self,
        scope: Scope,
        manager: &str,
        package_name: &str,
    ) -> Result<String> {
        let mut config = self.load_or_default(scope)?;
        let before = config.tracked_packages.len();
        config
            .tracked_packages
            .retain(|pkg| !(pkg.manager == manager && pkg.package_name == package_name));
        self.save_config(scope, &config)?;

        let removed = before.saturating_sub(config.tracked_packages.len());
        Ok(format!(
            "Stopped tracking package target in {} scope: {} {}. Removed {} record{}.",
            scope,
            manager,
            package_name,
            removed,
            pluralize(removed)
        ))
    }

    pub fn set_diff_mode(
        &self,
        scope: Scope,
        raw_path: &str,
        diff_mode: DiffMode,
    ) -> Result<String> {
        let mut config = self.load_or_default(scope)?;
        let path = paths::normalize_display_path(&paths::expand_path(raw_path)?);
        let updated = set_path_policy(&mut config, &path, |entry| entry.diff_mode = diff_mode);
        self.save_config(scope, &config)?;
        if updated {
            Ok(format!(
                "Updated diff mode for {} in {} scope to {}.",
                path,
                scope,
                diff_mode_label(diff_mode)
            ))
        } else {
            Err(anyhow!("{} is not tracked yet", path))
        }
    }

    pub fn set_redaction_mode(
        &self,
        scope: Scope,
        raw_path: &str,
        redaction: RedactionMode,
    ) -> Result<String> {
        let mut config = self.load_or_default(scope)?;
        let path = paths::normalize_display_path(&paths::expand_path(raw_path)?);
        let updated = set_path_policy(&mut config, &path, |entry| entry.redaction = redaction);
        self.save_config(scope, &config)?;
        if updated {
            Ok(format!(
                "Updated redaction mode for {} in {} scope to {}.",
                path,
                scope,
                redaction_label(redaction)
            ))
        } else {
            Err(anyhow!("{} is not tracked yet", path))
        }
    }

    pub fn service_action(&self, action: &str, scope: Scope) -> Result<String> {
        match action {
            "install" => self.install_service(scope),
            "start" => self.start_service(scope),
            "stop" => self.stop_service(scope),
            "status" => self.status_service(scope),
            _ => Err(anyhow!("Unknown service action.")),
        }
    }

    pub fn clear_history(&self, scope: Scope) -> Result<String> {
        let paths = self.paths_for_scope(scope);
        let journal_path = paths.journal_file();
        let daemon_state_path = paths.daemon_state_file();
        let mut removed = 0usize;

        if journal_path.exists() {
            fs::remove_file(&journal_path)
                .with_context(|| format!("failed to remove journal {}", journal_path.display()))?;
            removed += 1;
        }

        if daemon_state_path.exists() {
            fs::remove_file(&daemon_state_path).with_context(|| {
                format!(
                    "failed to remove daemon state {}",
                    daemon_state_path.display()
                )
            })?;
            removed += 1;
        }

        Ok(match removed {
            0 => format!("No {} history files were present to clear.", scope),
            _ => format!(
                "Cleared {} history. Removed {} file{} and reset the daemon baseline.",
                scope,
                removed,
                pluralize(removed)
            ),
        })
    }

    pub fn infer_scope_for_path(&self, raw_path: &str) -> Result<Option<Scope>> {
        Ok(paths::infer_scope_for_path(&paths::expand_path(raw_path)?))
    }

    pub(crate) fn load_setup_profile(&self) -> Result<Option<SetupProfile>> {
        let path = self.shared_setup_file();
        if !path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read setup profile {}", path.display()))?;
        let mut profile: SetupProfile = toml::from_str(&contents)
            .with_context(|| format!("failed to parse setup profile {}", path.display()))?;
        profile.sort_and_dedup();
        Ok(Some(profile))
    }

    pub(crate) fn load_or_default(&self, scope: Scope) -> Result<Config> {
        let config_path = self.paths_for_scope(scope).config_file();
        if config_path.exists() {
            self.load_config(scope)
        } else {
            Ok(Config::new())
        }
    }

    fn load_config(&self, scope: Scope) -> Result<Config> {
        let path = self.paths_for_scope(scope).config_file();
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let mut config: Config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        if config.version == 0 {
            config.version = 1;
        }
        config.sort_and_dedup();
        Ok(config)
    }

    fn save_config(&self, scope: Scope, config: &Config) -> Result<()> {
        let paths = self.paths_for_scope(scope);
        ensure_scope_directories(paths)?;
        let path = paths.config_file();
        let contents = toml::to_string_pretty(config).context("failed to serialize config")?;
        write_atomic(&path, "toml", contents.as_bytes(), scope)?;
        Ok(())
    }

    fn save_setup_profile(&self, profile: &SetupProfile) -> Result<()> {
        ensure_scope_directories(&self.system_paths)?;
        let path = self.shared_setup_file();
        let contents =
            toml::to_string_pretty(profile).context("failed to serialize setup profile")?;
        write_atomic_with_mode(&path, "toml", contents.as_bytes(), 0o644)?;
        Ok(())
    }

    pub(crate) fn load_daemon_state(&self, scope: Scope) -> Result<daemon::DaemonState> {
        let path = self.paths_for_scope(scope).daemon_state_file();
        if !path.exists() {
            return Ok(daemon::DaemonState::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read daemon state {}", path.display()))?;
        let state = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse daemon state {}", path.display()))?;
        Ok(state)
    }

    pub(crate) fn save_daemon_state(
        &self,
        scope: Scope,
        state: &daemon::DaemonState,
    ) -> Result<()> {
        let paths = self.paths_for_scope(scope);
        ensure_scope_directories(paths)?;
        let path = paths.daemon_state_file();
        let contents =
            serde_json::to_string_pretty(state).context("failed to serialize daemon state")?;
        write_atomic(&path, "json", contents.as_bytes(), scope)?;
        Ok(())
    }

    pub(crate) fn append_events(&self, scope: Scope, events: &[JournalEvent]) -> Result<()> {
        let paths = self.paths_for_scope(scope);
        ensure_scope_directories(paths)?;
        let path = paths.journal_file();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open journal {}", path.display()))?;
        set_scope_file_permissions(&path, scope)?;
        for event in events {
            let line = serde_json::to_string(event).context("failed to serialize journal event")?;
            file.write_all(line.as_bytes())
                .context("failed to write journal event")?;
            file.write_all(b"\n")
                .context("failed to write journal newline")?;
        }
        Ok(())
    }

    pub(crate) fn shared_setup_file(&self) -> PathBuf {
        self.system_paths.config_home.join("setup.toml")
    }

    fn filesystem_root(&self) -> PathBuf {
        self.system_paths
            .config_home
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    fn user_home_for_presets(&self) -> Option<PathBuf> {
        self.user_paths
            .config_home
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    fn apply_setup_presets_to_scope(
        &self,
        scope: Scope,
        include_setup_only: bool,
    ) -> Result<Vec<TrackedPath>> {
        let mut config = self.load_or_default(scope)?;
        config
            .tracked_paths
            .retain(|entry| entry.source != TrackSource::Preset);
        let applied = detect_presets(self, scope, include_setup_only)?;
        config.tracked_paths.extend(applied.clone());
        config.sort_and_dedup();
        self.save_config(scope, &config)?;
        Ok(applied)
    }

    pub(crate) fn load_events(&self, scope: Scope) -> Result<Vec<JournalEvent>> {
        let path = self.paths_for_scope(scope).journal_file();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        let file = fs::File::open(&path)
            .with_context(|| format!("failed to read journal {}", path.display()))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line =
                line.with_context(|| format!("failed to read journal line in {}", path.display()))?;
            if line.trim().is_empty() {
                continue;
            }
            let event: JournalEvent = serde_json::from_str(&line)
                .with_context(|| format!("failed to parse journal line in {}", path.display()))?;
            events.push(event);
        }
        Ok(events)
    }

    fn load_filtered_events(
        &self,
        scope: Scope,
        filters: render::CategoryFilters<'_>,
        path: Option<&str>,
        since: Option<OffsetDateTime>,
        until: Option<OffsetDateTime>,
        limit: Option<usize>,
    ) -> Result<Vec<JournalEvent>> {
        let journal_path = self.paths_for_scope(scope).journal_file();
        if !journal_path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&journal_path)
            .with_context(|| format!("failed to read journal {}", journal_path.display()))?;
        let reader = BufReader::new(file);

        let mut limited = limit.map(|_| VecDeque::new());
        let mut all_events = if limit.is_none() {
            Some(Vec::new())
        } else {
            None
        };

        for line in reader.lines() {
            let line = line.with_context(|| {
                format!("failed to read journal line in {}", journal_path.display())
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let event: JournalEvent = serde_json::from_str(&line).with_context(|| {
                format!("failed to parse journal line in {}", journal_path.display())
            })?;
            if !event_matches_filters(&event, filters, path, since, until) {
                continue;
            }

            if let Some(ref mut queue) = limited {
                queue.push_back(event);
                if let Some(max) = limit {
                    while queue.len() > max {
                        queue.pop_front();
                    }
                }
            } else if let Some(ref mut vec) = all_events {
                vec.push(event);
            }
        }

        if let Some(queue) = limited {
            Ok(queue.into_iter().collect())
        } else {
            Ok(all_events.unwrap_or_default())
        }
    }

    pub(crate) fn enforce_journal_retention(
        &self,
        scope: Scope,
        retention: &RetentionPolicy,
    ) -> Result<()> {
        let path = self.paths_for_scope(scope).journal_file();
        if !path.exists() {
            return Ok(());
        }

        let metadata = fs::metadata(&path)
            .with_context(|| format!("failed to read journal metadata {}", path.display()))?;
        let events = self.load_events(scope)?;
        let exceeds_events = events.len() > retention.max_events;
        let exceeds_bytes = metadata.len() > retention.max_bytes;
        if !exceeds_events && !exceeds_bytes {
            return Ok(());
        }

        let start_index = daemon::retention_start_index(&events, retention);
        let retained = &events[start_index..];
        let mut file = fs::File::create(&path)
            .with_context(|| format!("failed to truncate journal {}", path.display()))?;
        for event in retained {
            let line = serde_json::to_string(event)
                .context("failed to serialize retained journal event")?;
            file.write_all(line.as_bytes())
                .context("failed to write retained journal event")?;
            file.write_all(b"\n")
                .context("failed to write retained journal newline")?;
        }
        Ok(())
    }

    fn install_service(&self, scope: Scope) -> Result<String> {
        let unit_path = service::service_unit_install_path(scope)?;
        let unit_dir = unit_path
            .parent()
            .context("service unit path is missing a parent directory")?;
        fs::create_dir_all(unit_dir)
            .with_context(|| format!("failed to create unit directory {}", unit_dir.display()))?;

        let daemon_path = service::daemon_binary_path()?;
        let contents = service::render_systemd_unit(scope, &daemon_path);
        write_atomic_with_mode(&unit_path, "service", contents.as_bytes(), 0o644)?;

        service::run_systemctl(scope, ["daemon-reload"])?;

        Ok(format!(
            "Installed {} service unit at {}.\nRun `{} changedd` or `changed service start {}` next.",
            scope,
            unit_path.display(),
            if scope == Scope::System {
                "sudo systemctl enable --now"
            } else {
                "systemctl --user enable --now"
            },
            if scope == Scope::System { "-S" } else { "-U" }
        ))
    }

    fn start_service(&self, scope: Scope) -> Result<String> {
        service::run_systemctl(scope, ["daemon-reload"])?;
        service::run_systemctl(scope, ["enable", "--now", service::systemd_unit_name()])?;
        Ok(format!("Started and enabled {} service.", scope))
    }

    fn stop_service(&self, scope: Scope) -> Result<String> {
        service::run_systemctl(scope, ["disable", "--now", service::systemd_unit_name()])?;
        Ok(format!("Stopped and disabled {} service.", scope))
    }

    fn status_service(&self, scope: Scope) -> Result<String> {
        service::run_systemctl(
            scope,
            [
                "status",
                service::systemd_unit_name(),
                "--no-pager",
                "--full",
            ],
        )
    }
}

fn event_matches_filters(
    event: &JournalEvent,
    filters: render::CategoryFilters<'_>,
    path: Option<&str>,
    since: Option<OffsetDateTime>,
    until: Option<OffsetDateTime>,
) -> bool {
    filters.matches(event.category)
        && path.is_none_or(|wanted| wanted == event.path)
        && since.is_none_or(|value| event.timestamp >= value)
        && until.is_none_or(|value| event.timestamp <= value)
}

fn parse_filter_time(value: Option<&str>) -> Result<Option<OffsetDateTime>> {
    match value {
        Some(value) => {
            let parsed = OffsetDateTime::parse(value, &Rfc3339)
                .with_context(|| format!("failed to parse time '{value}' as RFC3339"))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

fn set_path_policy<F>(config: &mut Config, path: &str, mut update: F) -> bool
where
    F: FnMut(&mut TrackedPath),
{
    let mut updated = false;
    for entry in &mut config.tracked_paths {
        if entry.path == path {
            update(entry);
            updated = true;
        }
    }
    if updated {
        config.sort_and_dedup();
    }
    updated
}

fn upsert_path(config: &mut Config, path: TrackedPath) {
    config.tracked_paths.retain(|entry| entry.path != path.path);
    config.tracked_paths.push(path);
    config.sort_and_dedup();
}

fn detect_presets(app: &App, scope: Scope, include_setup_only: bool) -> Result<Vec<TrackedPath>> {
    let mut all = Vec::new();
    for category in Category::ALL {
        if category == Category::Packages {
            continue;
        }
        if !include_setup_only && is_setup_only_category(category) {
            continue;
        }
        all.extend(preset_targets_for_category(app, scope, category)?);
    }
    Ok(all)
}

fn preset_targets_for_category(
    app: &App,
    scope: Scope,
    category: Category,
) -> Result<Vec<TrackedPath>> {
    let home = app.user_home_for_presets();
    let root = app.filesystem_root();

    let candidates = match category {
        Category::Cpu => vec![
            Some(preset_file(
                root_join(&root, "/etc/default/cpupower"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/modprobe.d/amd-pstate.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/intel-undervolt.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/default/cpupower-service.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/systemd/system/intel-pstate-pin.service"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/gamemode.ini"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
        ],
        Category::Gpu => vec![
            Some(preset_file(
                root_join(&root, "/etc/modprobe.d/nvidia.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/udev/rules.d/99-nvidia-irq-affinity.rules"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/nvidia_oc.json"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/X11/xorg.conf.d/20-amdgpu.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/X11/xorg.conf.d/20-intel.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            home.as_ref().map(|home| {
                preset_dir(
                    home.join(".config/hypr"),
                    category,
                    DiffMode::Unified,
                    RedactionMode::Off,
                )
            }),
        ],
        Category::Services => vec![
            Some(preset_file(
                root_join(&root, "/etc/systemd/system.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/environment"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/dhcpcd.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/sudo.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            home.as_ref().map(|home| {
                preset_dir(
                    home.join(".config/systemd/user"),
                    category,
                    DiffMode::MetadataOnly,
                    RedactionMode::Off,
                )
            }),
        ],
        Category::Scheduler => vec![
            Some(preset_file(
                root_join(&root, "/etc/sysctl.d/99-scheduler.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/udev/rules.d/60-ioschedulers.rules"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
        ],
        Category::Shell => vec![
            home.as_ref().map(|home| {
                preset_file(
                    home.join(".config/fish/config.fish"),
                    category,
                    DiffMode::MetadataOnly,
                    RedactionMode::Auto,
                )
            }),
            home.as_ref().map(|home| {
                preset_file(
                    home.join(".bashrc"),
                    category,
                    DiffMode::MetadataOnly,
                    RedactionMode::Auto,
                )
            }),
            home.as_ref().map(|home| {
                preset_file(
                    home.join(".zshrc"),
                    category,
                    DiffMode::MetadataOnly,
                    RedactionMode::Auto,
                )
            }),
            Some(preset_file(
                root_join(&root, "/etc/bash.bashrc"),
                category,
                DiffMode::MetadataOnly,
                RedactionMode::Auto,
            )),
        ],
        Category::Build => vec![
            Some(preset_file(
                root_join(&root, "/etc/makepkg.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            home.as_ref().map(|home| {
                preset_file(
                    home.join(".config/pacman/makepkg.conf"),
                    category,
                    DiffMode::Unified,
                    RedactionMode::Off,
                )
            }),
            Some(preset_file(
                root_join(&root, "/etc/ccache.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/pacman.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_dir(
                root_join(&root, "/etc/makepkg.conf.d"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_dir(
                root_join(&root, "/etc/makepkg.d"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
        ],
        Category::Boot => vec![
            Some(preset_file(
                root_join(&root, "/etc/fstab"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/default/grub"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_file(
                root_join(&root, "/etc/mkinitcpio.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
            Some(preset_dir(
                root_join(&root, "/boot/loader/entries"),
                category,
                DiffMode::MetadataOnly,
                RedactionMode::Off,
            )),
        ],
        Category::Audio => vec![
            home.as_ref().map(|home| {
                preset_file(
                    home.join(".config/pipewire/pipewire.conf"),
                    category,
                    DiffMode::Unified,
                    RedactionMode::Off,
                )
            }),
            home.as_ref().map(|home| {
                preset_dir(
                    home.join(".config/wireplumber/wireplumber.conf.d"),
                    category,
                    DiffMode::MetadataOnly,
                    RedactionMode::Off,
                )
            }),
            Some(preset_file(
                root_join(&root, "/etc/pipewire/pipewire.conf"),
                category,
                DiffMode::Unified,
                RedactionMode::Off,
            )),
        ],
        Category::Packages => Vec::new(),
    };

    Ok(candidates
        .into_iter()
        .flatten()
        .filter(|entry| preset_matches_scope(app, scope, Path::new(&entry.path)))
        .filter(|entry| Path::new(&entry.path).exists())
        .collect())
}

fn is_setup_only_category(category: Category) -> bool {
    matches!(category, Category::Cpu | Category::Gpu | Category::Build)
}

fn root_join(root: &Path, absolute: &str) -> PathBuf {
    let relative = absolute.strip_prefix('/').unwrap_or(absolute);
    if root == Path::new("/") {
        PathBuf::from(absolute)
    } else {
        root.join(relative)
    }
}

fn preset_matches_scope(app: &App, scope: Scope, path: &Path) -> bool {
    let user_home = app.user_home_for_presets();
    let system_root = app.filesystem_root();
    let system_etc = root_join(&system_root, "/etc");
    let system_boot = root_join(&system_root, "/boot");

    match scope {
        Scope::User => user_home
            .as_ref()
            .is_some_and(|home| path.starts_with(home)),
        Scope::System => path.starts_with(&system_etc) || path.starts_with(&system_boot),
    }
}

fn preset_file<P: Into<PathBuf>>(
    path: P,
    category: Category,
    diff_mode: DiffMode,
    redaction: RedactionMode,
) -> TrackedPath {
    let path = path.into();
    TrackedPath {
        path: paths::normalize_display_path(&path),
        category,
        kind: PathKind::File,
        diff_mode,
        redaction,
        source: TrackSource::Preset,
    }
}

fn preset_dir<P: Into<PathBuf>>(
    path: P,
    category: Category,
    diff_mode: DiffMode,
    redaction: RedactionMode,
) -> TrackedPath {
    let path = path.into();
    TrackedPath {
        path: paths::normalize_display_path(&path),
        category,
        kind: PathKind::Directory,
        diff_mode,
        redaction,
        source: TrackSource::Preset,
    }
}

fn infer_category_for_path(path: &Path) -> Category {
    let path_str = paths::normalize_display_path(path);
    if path_str.contains("makepkg") {
        Category::Build
    } else if path_str.contains("fish")
        || path_str.ends_with(".bashrc")
        || path_str.ends_with(".zshrc")
    {
        Category::Shell
    } else if path_str.contains("systemd") || path_str.ends_with(".service") {
        Category::Services
    } else if path_str.contains("grub")
        || path_str.contains("mkinitcpio")
        || path_str.contains("/boot/")
    {
        Category::Boot
    } else if path_str.contains("pipewire")
        || path_str.contains("wireplumber")
        || path_str.contains("alsa")
    {
        Category::Audio
    } else if path_str.contains("sysctl") || path_str.contains("scheduler") {
        Category::Scheduler
    } else if path_str.contains("nvidia")
        || path_str.contains("amdgpu")
        || path_str.contains("/X11/")
    {
        Category::Gpu
    } else if path_str.contains("cpupower")
        || path_str.contains("pstate")
        || path_str.contains("cpu")
    {
        Category::Cpu
    } else {
        Category::Services
    }
}

fn default_diff_mode_for_category(category: Category) -> DiffMode {
    match category {
        Category::Shell => DiffMode::MetadataOnly,
        _ => DiffMode::Unified,
    }
}

fn default_redaction_for_category(category: Category) -> RedactionMode {
    match category {
        Category::Shell => RedactionMode::Auto,
        _ => RedactionMode::Off,
    }
}

fn diff_mode_label(mode: DiffMode) -> &'static str {
    match mode {
        DiffMode::MetadataOnly => "metadata-only",
        DiffMode::Unified => "unified diff",
    }
}

fn redaction_label(mode: RedactionMode) -> &'static str {
    match mode {
        RedactionMode::Off => "off",
        RedactionMode::Auto => "auto",
    }
}

fn pluralize(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn render_setup_summary(setup_file: &Path, system: &[TrackedPath], user: &[TrackedPath]) -> String {
    let mut out = String::new();
    out.push_str("Setup complete.\n");
    out.push_str(&format!(
        "Shared setup profile: {}\n\n",
        setup_file.display()
    ));
    out.push_str("Files successfully tracked:\n\n");
    render_setup_scope(&mut out, "System", system);
    out.push('\n');
    render_setup_scope(&mut out, "User", user);
    out.push('\n');
    out.push_str(
        "If you want other files added, please use `changed track [args]` to add custom paths.\n",
    );

    let system_activity = service::query_service_activity(Scope::System);
    let user_activity = service::query_service_activity(Scope::User);
    if let Some(note) = render_setup_service_note(system_activity, user_activity) {
        out.push('\n');
        out.push_str(&note);
        out.push('\n');
    }

    out.trim_end().to_owned()
}

fn render_setup_service_note(
    system_activity: service::ServiceActivity,
    user_activity: service::ServiceActivity,
) -> Option<String> {
    let mut lines = Vec::new();

    if user_activity == service::ServiceActivity::Inactive {
        lines.push(String::from(
            "- User scope is not currently running. Run `systemctl --user enable --now changedd.service` to start tracking user files.",
        ));
    }

    if system_activity == service::ServiceActivity::Inactive {
        lines.push(String::from(
            "- System scope is not currently running. Run `sudo systemctl enable --now changedd.service` to start tracking system files.",
        ));
    }

    if lines.is_empty() {
        None
    } else {
        Some(format!("NOTE:\n{}", lines.join("\n")))
    }
}

fn render_setup_scope(out: &mut String, label: &str, entries: &[TrackedPath]) {
    out.push_str(label);
    out.push_str(":\n");
    if entries.is_empty() {
        out.push_str("- none\n");
        return;
    }

    for entry in entries {
        out.push_str("- ");
        out.push_str(&entry.path);
        out.push('\n');
    }
}

fn detect_path_kind(path: &Path) -> PathKind {
    if path.is_dir() {
        PathKind::Directory
    } else {
        PathKind::File
    }
}

fn ensure_scope_directories(paths: &AppPaths) -> Result<()> {
    fs::create_dir_all(&paths.config_home).with_context(|| {
        format!(
            "failed to create config directory {}",
            paths.config_home.display()
        )
    })?;
    fs::create_dir_all(&paths.state_home).with_context(|| {
        format!(
            "failed to create state directory {}",
            paths.state_home.display()
        )
    })?;
    set_scope_path_owner(&paths.config_home, paths.scope)?;
    set_scope_path_owner(&paths.state_home, paths.scope)?;
    set_scope_config_dir_permissions(&paths.config_home, paths.scope)?;
    set_scope_state_dir_permissions(&paths.state_home)?;
    Ok(())
}

fn write_atomic(path: &Path, extension: &str, contents: &[u8], scope: Scope) -> Result<()> {
    let temp_path = path.with_extension(format!(
        "{extension}.tmp.{}",
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    fs::write(&temp_path, contents)
        .with_context(|| format!("failed to write temporary file {}", temp_path.display()))?;
    set_scope_path_owner(&temp_path, scope)?;
    set_scope_file_permissions(&temp_path, scope)?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            temp_path.display()
        )
    })?;
    set_scope_path_owner(path, scope)?;
    set_scope_file_permissions(path, scope)?;
    Ok(())
}

fn write_atomic_with_mode(path: &Path, extension: &str, contents: &[u8], mode: u32) -> Result<()> {
    let temp_path = path.with_extension(format!(
        "{extension}.tmp.{}",
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    fs::write(&temp_path, contents)
        .with_context(|| format!("failed to write temporary file {}", temp_path.display()))?;
    set_mode(&temp_path, mode)?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            temp_path.display()
        )
    })?;
    set_mode(path, mode)?;
    Ok(())
}

#[cfg(unix)]
fn set_scope_config_dir_permissions(path: &Path, scope: Scope) -> Result<()> {
    let mode = match scope {
        Scope::System => 0o755,
        Scope::User => 0o700,
    };
    set_mode(path, mode)
}

#[cfg(not(unix))]
fn set_scope_config_dir_permissions(_path: &Path, _scope: Scope) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_scope_state_dir_permissions(path: &Path) -> Result<()> {
    set_mode(path, 0o700)
}

#[cfg(not(unix))]
fn set_scope_state_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_scope_file_permissions(path: &Path, scope: Scope) -> Result<()> {
    let mode = match scope {
        Scope::System => 0o600,
        Scope::User => 0o600,
    };
    set_mode(path, mode)
}

#[cfg(not(unix))]
fn set_scope_file_permissions(_path: &Path, _scope: Scope) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_scope_path_owner(path: &Path, scope: Scope) -> Result<()> {
    use std::os::unix::fs::chown;

    if scope != Scope::User || !nix::unistd::Uid::effective().is_root() {
        return Ok(());
    }

    let Some((uid, gid)) = paths::sudo_user_owner()? else {
        return Ok(());
    };

    chown(path, Some(uid.as_raw()), Some(gid.as_raw()))
        .with_context(|| format!("failed to set ownership on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_scope_path_owner(_path: &Path, _scope: Scope) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::EventKind;
    use std::collections::BTreeMap;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestEnv {
        root: PathBuf,
    }

    impl TestEnv {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before unix epoch")
                .as_nanos();
            let root = env::temp_dir().join(format!("changed-test-{unique}"));
            fs::create_dir_all(&root).expect("temp root should be creatable");
            Self { root }
        }

        fn app(&self) -> App {
            App {
                user_paths: AppPaths {
                    scope: Scope::User,
                    config_home: self.root.join("home/.config/changed"),
                    state_home: self.root.join("home/.local/state/changed"),
                },
                system_paths: AppPaths {
                    scope: Scope::System,
                    config_home: self.root.join("etc/changed"),
                    state_home: self.root.join("var/lib/changed"),
                },
            }
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn track_file_persists_config() {
        let env = TestEnv::new();
        let app = env.app();

        app.track_file(Scope::System, "/etc/makepkg.conf")
            .expect("tracking should succeed");
        let config = app.load_config(Scope::System).expect("config should load");

        assert_eq!(config.tracked_paths.len(), 1);
        assert_eq!(config.tracked_paths[0].category, Category::Build);
        assert_eq!(config.tracked_paths[0].diff_mode, DiffMode::Unified);
    }

    #[test]
    fn infer_scope_for_path_distinguishes_home_and_system_paths() {
        let env = TestEnv::new();
        let app = env.app();
        let user_path = paths::home_dir()
            .expect("home should exist")
            .join(".config/changed-test/config.fish");

        assert_eq!(
            app.infer_scope_for_path(user_path.to_string_lossy().as_ref())
                .expect("inference should succeed"),
            Some(Scope::User)
        );
        assert_eq!(
            app.infer_scope_for_path("/etc/makepkg.conf")
                .expect("inference should succeed"),
            Some(Scope::System)
        );
    }

    #[test]
    fn user_state_path_default_does_not_duplicate_local_state_segment() {
        let paths = AppPaths::detect(Scope::User).expect("user paths should resolve");
        let rendered = paths.state_home.to_string_lossy();

        assert!(rendered.ends_with("/changed"));
        assert!(!rendered.contains(".local/state/.local/state"));
    }

    #[test]
    fn track_category_adds_matching_presets() {
        let env = TestEnv::new();
        let app = env.app();
        let fish_dir = env.root.join("home/.config/fish");
        fs::create_dir_all(&fish_dir).expect("fish config dir should be creatable");
        let fish_config = fish_dir.join("config.fish");
        if !fish_config.exists() {
            fs::write(&fish_config, "# test\n").expect("fish config should be writable");
        }

        app.track_category(Scope::User, Category::Shell)
            .expect("category tracking should succeed");
        let config = app.load_config(Scope::User).expect("config should load");
        assert!(
            config
                .tracked_paths
                .iter()
                .any(|entry| entry.category == Category::Shell)
        );
    }

    #[test]
    fn setup_writes_shared_profile_and_applies_existing_setup_candidates() {
        let env = TestEnv::new();
        let app = env.app();

        fs::create_dir_all(env.root.join("etc/modprobe.d")).expect("modprobe dir should exist");
        fs::create_dir_all(env.root.join("etc/udev/rules.d")).expect("udev dir should exist");
        fs::create_dir_all(env.root.join("etc/systemd/system")).expect("systemd dir should exist");
        fs::create_dir_all(env.root.join("etc/default")).expect("default dir should exist");
        fs::create_dir_all(env.root.join("home/.config/pacman"))
            .expect("pacman config dir should exist");
        fs::create_dir_all(env.root.join("home/.config/fish"))
            .expect("fish config dir should exist");
        fs::create_dir_all(env.root.join("home/.config/hypr/conf.d"))
            .expect("hypr config dir should exist");

        fs::write(
            env.root.join("etc/modprobe.d/nvidia.conf"),
            "options nvidia modeset=1\n",
        )
        .expect("nvidia modprobe config should exist");
        fs::write(
            env.root
                .join("etc/udev/rules.d/99-nvidia-irq-affinity.rules"),
            "ACTION==\"add\"\n",
        )
        .expect("nvidia irq affinity rule should exist");
        fs::write(
            env.root.join("etc/nvidia_oc.json"),
            "{ \"sets\": { \"0\": {} } }\n",
        )
        .expect("nvidia oc json should exist");
        fs::write(
            env.root.join("etc/intel-undervolt.conf"),
            "undervolt 0 'CPU' 0\n",
        )
        .expect("intel undervolt config should exist");
        fs::write(
            env.root.join("etc/default/cpupower-service.conf"),
            "GOVERNOR='performance'\n",
        )
        .expect("cpupower service config should exist");
        fs::write(
            env.root.join("etc/systemd/system/intel-pstate-pin.service"),
            "[Service]\nExecStart=/bin/true\n",
        )
        .expect("intel pstate service should exist");
        fs::write(
            env.root.join("home/.config/pacman/makepkg.conf"),
            "MAKEFLAGS='-j16'\n",
        )
        .expect("user makepkg config should exist");
        fs::write(
            env.root.join("home/.config/fish/config.fish"),
            "set -g fish_greeting \"\"\n",
        )
        .expect("fish config should exist");
        fs::write(
            env.root.join("home/.config/hypr/hyprland.conf"),
            "$mod = SUPER\n",
        )
        .expect("hypr config should exist");

        let output = app
            .setup_with_profile(crate::setup::SetupProfile {
                version: 1,
                cpu_vendor: Some(crate::setup::CpuVendor::Intel),
                gpu_vendors: vec![crate::setup::GpuVendor::Nvidia],
                shells: vec![crate::setup::ShellKind::Fish],
            })
            .expect("setup should succeed");
        assert!(output.contains("Files successfully tracked:"));
        assert!(output.contains("System:"));
        assert!(output.contains("User:"));
        assert!(output.contains("/etc/modprobe.d/nvidia.conf"));
        assert!(output.contains(".config/pacman/makepkg.conf"));
        assert!(output.contains(".config/fish/config.fish"));
        assert!(output.contains(".config/hypr"));
        assert!(output.contains("changed track [args]"));

        let saved = app
            .load_setup_profile()
            .expect("setup profile should load")
            .expect("setup profile should exist");
        assert_eq!(saved.cpu_vendor, Some(crate::setup::CpuVendor::Intel));
        assert_eq!(saved.gpu_vendors, vec![crate::setup::GpuVendor::Nvidia]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let setup_mode = fs::metadata(app.shared_setup_file())
                .expect("setup profile should exist")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(setup_mode, 0o644);
        }

        let system_config = app
            .load_config(Scope::System)
            .expect("system config should load");
        assert!(
            system_config
                .tracked_paths
                .iter()
                .any(|entry| entry.path.ends_with("/etc/intel-undervolt.conf"))
        );
        assert!(
            system_config
                .tracked_paths
                .iter()
                .any(|entry| entry.path.ends_with("/etc/modprobe.d/nvidia.conf"))
        );
        assert!(
            system_config
                .tracked_paths
                .iter()
                .any(|entry| entry.path.ends_with("/etc/nvidia_oc.json"))
        );

        let user_config = app
            .load_config(Scope::User)
            .expect("user config should load");
        assert!(
            user_config
                .tracked_paths
                .iter()
                .any(|entry| entry.path.ends_with(".config/pacman/makepkg.conf"))
        );
        assert!(
            user_config
                .tracked_paths
                .iter()
                .any(|entry| entry.path.ends_with(".config/fish/config.fish"))
        );
        assert!(
            user_config
                .tracked_paths
                .iter()
                .any(|entry| entry.path.ends_with(".config/hypr"))
        );
    }

    #[test]
    fn init_skips_setup_only_presets_until_setup_profile_exists() {
        let env = TestEnv::new();
        let app = env.app();

        fs::create_dir_all(env.root.join("etc")).expect("etc dir should exist");
        fs::write(env.root.join("etc/makepkg.conf"), "MAKEFLAGS='-j16'\n")
            .expect("system makepkg config should exist");

        app.init(Scope::System).expect("init should succeed");
        let config = app.load_config(Scope::System).expect("config should load");

        assert!(
            !config
                .tracked_paths
                .iter()
                .any(|entry| entry.path.ends_with("/etc/makepkg.conf"))
        );
    }

    #[test]
    fn setup_service_note_reports_each_inactive_scope() {
        let note = render_setup_service_note(
            service::ServiceActivity::Inactive,
            service::ServiceActivity::Active,
        )
        .expect("system-only note should exist");
        assert!(note.contains("System scope is not currently running"));
        assert!(!note.contains("User scope is not currently running"));

        let note = render_setup_service_note(
            service::ServiceActivity::Active,
            service::ServiceActivity::Inactive,
        )
        .expect("user-only note should exist");
        assert!(note.contains("User scope is not currently running"));
        assert!(!note.contains("System scope is not currently running"));

        let note = render_setup_service_note(
            service::ServiceActivity::Inactive,
            service::ServiceActivity::Inactive,
        )
        .expect("dual-scope note should exist");
        assert!(note.contains("User scope is not currently running"));
        assert!(note.contains("System scope is not currently running"));

        assert!(
            render_setup_service_note(
                service::ServiceActivity::Active,
                service::ServiceActivity::Active
            )
            .is_none()
        );
    }

    #[test]
    fn track_file_rejects_missing_path() {
        let env = TestEnv::new();
        let app = env.app();
        let missing = env.root.join("does-not-exist.conf");

        let error = app
            .track_file(Scope::User, missing.to_string_lossy().as_ref())
            .expect_err("missing path should fail");

        assert!(error.to_string().contains("File not found"));
        let config = app
            .load_or_default(Scope::User)
            .expect("config should load");
        assert!(config.tracked_paths.is_empty());
    }

    #[test]
    fn tracked_listing_filters_include_and_exclude_categories() {
        let env = TestEnv::new();
        let app = env.app();
        let user_file = paths::home_dir()
            .expect("home should exist")
            .join(".config/fish/config.fish");

        app.track_file(Scope::System, "/etc/makepkg.conf")
            .expect("system tracking should succeed");
        app.track_file(Scope::User, user_file.to_string_lossy().as_ref())
            .expect("user tracking should succeed");

        let output = app
            .list_tracked(
                &[Scope::System, Scope::User],
                &[Category::Build, Category::Shell],
                &[Category::Build],
                None,
                false,
            )
            .expect("tracked listing should succeed");

        assert!(output.contains("user:"));
        assert!(output.contains("shell:"));
        assert!(!output.contains("build:"));
        assert!(!output.contains("/etc/makepkg.conf"));
    }

    #[test]
    fn merged_history_includes_user_and_system_scopes_in_time_order() {
        let env = TestEnv::new();
        let app = env.app();

        app.append_events(
            Scope::System,
            &[JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T10:00:00Z", &Rfc3339)
                    .expect("timestamp should parse"),
                scope: Scope::System,
                kind: EventKind::Modified,
                category: Category::Boot,
                path: "/boot/loader/entries/arch.conf".to_owned(),
                summary: "Changed boot config (+1)".to_owned(),
                added_lines: 1,
                removed_lines: 0,
                diff: Some("(+) options mitigations=off".to_owned()),
            }],
        )
        .expect("system event append should succeed");

        app.append_events(
            Scope::User,
            &[JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T11:00:00Z", &Rfc3339)
                    .expect("timestamp should parse"),
                scope: Scope::User,
                kind: EventKind::Modified,
                category: Category::Shell,
                path: "/home/test/.config/fish/config.fish".to_owned(),
                summary: "Changed shell config".to_owned(),
                added_lines: 0,
                removed_lines: 0,
                diff: None,
            }],
        )
        .expect("user event append should succeed");

        let output = app
            .list_history(HistoryQuery {
                scopes: &[Scope::System, Scope::User],
                include: &[],
                exclude: &[],
                path: None,
                all: true,
                since: None,
                until: None,
                clean: true,
                color: false,
            })
            .expect("merged history should render");

        let boot_index = output
            .find("[system/boot]")
            .expect("system event should be present");
        let shell_index = output
            .find("[user/shell]")
            .expect("user event should be present");
        assert!(boot_index < shell_index);
    }

    #[test]
    fn merged_history_respects_include_and_exclude_filters() {
        let env = TestEnv::new();
        let app = env.app();

        let events = [
            JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T10:00:00Z", &Rfc3339)
                    .expect("timestamp should parse"),
                scope: Scope::System,
                kind: EventKind::Modified,
                category: Category::Boot,
                path: "/boot/loader/entries/arch.conf".to_owned(),
                summary: "Changed boot config (+1)".to_owned(),
                added_lines: 1,
                removed_lines: 0,
                diff: Some("(+) options mitigations=off".to_owned()),
            },
            JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T11:00:00Z", &Rfc3339)
                    .expect("timestamp should parse"),
                scope: Scope::User,
                kind: EventKind::Modified,
                category: Category::Shell,
                path: "/home/test/.config/fish/config.fish".to_owned(),
                summary: "Changed shell config".to_owned(),
                added_lines: 0,
                removed_lines: 0,
                diff: None,
            },
        ];

        app.append_events(Scope::System, &events[..1])
            .expect("system event append should succeed");
        app.append_events(Scope::User, &events[1..])
            .expect("user event append should succeed");

        let output = app
            .list_history(HistoryQuery {
                scopes: &[Scope::System, Scope::User],
                include: &[Category::Boot, Category::Shell],
                exclude: &[Category::Boot],
                path: None,
                all: true,
                since: None,
                until: None,
                clean: true,
                color: false,
            })
            .expect("filtered merged history should render");

        assert!(output.contains("[user/shell]"));
        assert!(!output.contains("[system/boot]"));
    }

    #[test]
    fn rendered_systemd_units_match_scope_flags() {
        let system = service::render_systemd_unit(Scope::System, Path::new("/usr/bin/changedd"));
        let user = service::render_systemd_unit(Scope::User, Path::new("/usr/bin/changedd"));

        assert!(system.contains("ExecStart=/usr/bin/changedd --system"));
        assert!(system.contains("WantedBy=multi-user.target"));
        assert!(user.contains("ExecStart=/usr/bin/changedd --user"));
        assert!(user.contains("WantedBy=default.target"));
    }

    #[test]
    fn diff_update_requires_existing_path() {
        let env = TestEnv::new();
        let app = env.app();

        let error = app
            .set_diff_mode(Scope::System, "/does/not/exist", DiffMode::Unified)
            .expect_err("missing tracked path should fail");
        assert!(error.to_string().contains("is not tracked yet"));
    }

    #[test]
    fn redaction_masks_sensitive_assignments() {
        let text = "export API_KEY=abc123\nset -gx SESSION_TOKEN xyz\nurl=https://user:pass@example.com\nrequest=Authorization: Bearer abcdefghijklmnop\ncallback=https://example.com?token=abc123&plain=value\nclient_secret = supersecret\n-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n-----END OPENSSH PRIVATE KEY-----\nPLAIN_VAR=value";
        let redacted = daemon::maybe_redact_text(text.to_owned(), RedactionMode::Auto);
        assert!(redacted.contains("API_KEY=[REDACTED]"));
        assert!(redacted.contains("set -gx SESSION_TOKEN [REDACTED]"));
        assert!(redacted.contains("https://[REDACTED]@example.com"));
        assert!(redacted.contains("Authorization: [REDACTED]"));
        assert!(redacted.contains("?token=[REDACTED]&plain=value"));
        assert!(redacted.contains("client_secret = [REDACTED]"));
        assert!(redacted.contains("[REDACTED PRIVATE KEY BLOCK]"));
        assert!(redacted.contains("PLAIN_VAR=value"));
    }

    #[test]
    fn clear_history_removes_journal_and_daemon_state() {
        let env = TestEnv::new();
        let app = env.app();

        app.append_events(
            Scope::User,
            &[JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T10:00:00Z", &Rfc3339)
                    .expect("timestamp should parse"),
                scope: Scope::User,
                kind: EventKind::Modified,
                category: Category::Shell,
                path: "/tmp/test".to_owned(),
                summary: "Changed shell config".to_owned(),
                added_lines: 0,
                removed_lines: 0,
                diff: None,
            }],
        )
        .expect("user journal append should succeed");

        app.save_daemon_state(
            Scope::User,
            &daemon::DaemonState {
                observed: BTreeMap::new(),
            },
        )
        .expect("daemon state save should succeed");

        let message = app
            .clear_history(Scope::User)
            .expect("history clear should succeed");

        assert!(message.contains("Cleared user history"));
        assert!(!app.user_paths.journal_file().exists());
        assert!(!app.user_paths.daemon_state_file().exists());
    }

    #[test]
    fn history_render_groups_by_date() {
        let events = vec![
            JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T01:00:00Z", &Rfc3339)
                    .expect("timestamp should parse"),
                scope: Scope::System,
                kind: EventKind::Modified,
                category: Category::Build,
                path: "/etc/makepkg.conf".to_owned(),
                summary: "Changed build config (+1)".to_owned(),
                added_lines: 1,
                removed_lines: 0,
                diff: Some("(+) MAKEFLAGS=-j16".to_owned()),
            },
            JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-04T01:00:00Z", &Rfc3339)
                    .expect("timestamp should parse"),
                scope: Scope::User,
                kind: EventKind::Modified,
                category: Category::Shell,
                path: "/home/test/.config/fish/config.fish".to_owned(),
                summary: "Changed shell config".to_owned(),
                added_lines: 0,
                removed_lines: 0,
                diff: None,
            },
        ];

        let rendered = render::render_history(&events, false, None, false);
        assert!(rendered.contains("# Changes"));
        assert!(rendered.contains("## 04/03/26"));
        assert!(rendered.contains("## 04/04/26"));
    }

    #[test]
    fn clean_history_render_is_compact() {
        let events = vec![JournalEvent {
            timestamp: OffsetDateTime::parse("2026-04-03T01:00:00Z", &Rfc3339)
                .expect("timestamp should parse"),
            scope: Scope::System,
            kind: EventKind::Modified,
            category: Category::Build,
            path: "/etc/makepkg.conf".to_owned(),
            summary: "Changed build config (+2/-1)".to_owned(),
            added_lines: 2,
            removed_lines: 1,
            diff: Some("(-) MAKEFLAGS=-j8\n(+) MAKEFLAGS=-j16".to_owned()),
        }];

        let rendered = render::render_history(&events, true, None, false);
        assert!(
            rendered.contains(
                "- 1:00am [system/build] /etc/makepkg.conf: Changed build config (+2/-1)"
            )
        );
        assert!(!rendered.contains("(+) MAKEFLAGS"));
    }

    #[test]
    fn clean_history_can_render_with_color() {
        let events = vec![JournalEvent {
            timestamp: OffsetDateTime::parse("2026-04-03T01:00:00Z", &Rfc3339)
                .expect("timestamp should parse"),
            scope: Scope::System,
            kind: EventKind::Modified,
            category: Category::Build,
            path: "/etc/makepkg.conf".to_owned(),
            summary: "Changed build config (+2/-1)".to_owned(),
            added_lines: 2,
            removed_lines: 1,
            diff: Some("(-) MAKEFLAGS=-j8\n(+) MAKEFLAGS=-j16".to_owned()),
        }];

        let rendered = render::render_history(&events, true, None, true);
        assert!(rendered.contains("\u{1b}["));
        assert!(rendered.contains("Changed build config (+2/-1)"));
    }

    #[test]
    fn first_daemon_scan_captures_baseline_without_events() {
        let env = TestEnv::new();
        let app = env.app();
        let tracked = env.root.join("tracked.conf");
        fs::write(&tracked, "value=1\n").expect("tracked file should be writable");

        app.track_file(Scope::User, tracked.to_string_lossy().as_ref())
            .expect("tracking should succeed");
        let message = app
            .run_daemon(Scope::User, DaemonOptions { once: true })
            .expect("daemon run should succeed");

        assert!(message.contains("Baseline captured"));
        let events = app.load_events(Scope::User).expect("events should load");
        assert!(events.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn scope_files_are_written_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let env = TestEnv::new();
        let app = env.app();

        app.init(Scope::User).expect("user init should succeed");
        app.init(Scope::System).expect("system init should succeed");

        let user_config_mode = fs::metadata(app.user_paths.config_file())
            .expect("user config should exist")
            .permissions()
            .mode()
            & 0o777;
        let user_state_mode = fs::metadata(app.user_paths.state_home.join("journal.jsonl"))
            .or_else(|_| {
                app.append_events(
                    Scope::User,
                    &[JournalEvent {
                        timestamp: OffsetDateTime::parse("2026-04-03T10:00:00Z", &Rfc3339)
                            .expect("timestamp should parse"),
                        scope: Scope::User,
                        kind: EventKind::Modified,
                        category: Category::Shell,
                        path: "/tmp/test".to_owned(),
                        summary: "Changed shell config".to_owned(),
                        added_lines: 0,
                        removed_lines: 0,
                        diff: None,
                    }],
                )
                .expect("user journal append should succeed");
                fs::metadata(app.user_paths.state_home.join("journal.jsonl"))
            })
            .expect("user journal should exist")
            .permissions()
            .mode()
            & 0o777;

        let system_config_mode = fs::metadata(app.system_paths.config_file())
            .expect("system config should exist")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(user_config_mode, 0o600);
        assert_eq!(user_state_mode, 0o600);
        assert_eq!(system_config_mode, 0o600);

        let user_dir_mode = fs::metadata(&app.user_paths.state_home)
            .expect("user state dir should exist")
            .permissions()
            .mode()
            & 0o777;
        let system_state_dir_mode = fs::metadata(&app.system_paths.state_home)
            .expect("system state dir should exist")
            .permissions()
            .mode()
            & 0o777;
        let system_config_dir_mode = fs::metadata(&app.system_paths.config_home)
            .expect("system config dir should exist")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(user_dir_mode, 0o700);
        assert_eq!(system_state_dir_mode, 0o700);
        assert_eq!(system_config_dir_mode, 0o755);
    }

    #[test]
    fn retention_start_index_respects_event_limit() {
        let events = (0..5)
            .map(|index| JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T01:00:00Z", &Rfc3339)
                    .expect("timestamp should parse")
                    + time::Duration::minutes(index),
                scope: Scope::System,
                kind: EventKind::Modified,
                category: Category::Build,
                path: format!("/tmp/{index}.conf"),
                summary: "Changed build config (+1)".to_owned(),
                added_lines: 1,
                removed_lines: 0,
                diff: Some("(+) X=1".to_owned()),
            })
            .collect::<Vec<_>>();

        let retention = RetentionPolicy {
            max_events: 2,
            max_bytes: 1024 * 1024,
        };
        let start = daemon::retention_start_index(&events, &retention);
        assert_eq!(start, 3);
    }

    #[test]
    fn merge_reloaded_observed_preserves_existing_and_baselines_new() {
        let previous = BTreeMap::from([(
            "/tmp/one.conf".to_owned(),
            daemon::ObservedPath {
                scope: Scope::System,
                path: "/tmp/one.conf".to_owned(),
                category: Category::Services,
                diff_mode: DiffMode::Unified,
                redaction: RedactionMode::Off,
                exists: true,
                fingerprint: Some("old".to_owned()),
                text_snapshot: Some("a=1".to_owned()),
            },
        )]);

        let current = BTreeMap::from([
            (
                "/tmp/one.conf".to_owned(),
                daemon::ObservedPath {
                    scope: Scope::System,
                    path: "/tmp/one.conf".to_owned(),
                    category: Category::Services,
                    diff_mode: DiffMode::Unified,
                    redaction: RedactionMode::Off,
                    exists: true,
                    fingerprint: Some("new".to_owned()),
                    text_snapshot: Some("a=2".to_owned()),
                },
            ),
            (
                "/tmp/two.conf".to_owned(),
                daemon::ObservedPath {
                    scope: Scope::System,
                    path: "/tmp/two.conf".to_owned(),
                    category: Category::Services,
                    diff_mode: DiffMode::Unified,
                    redaction: RedactionMode::Off,
                    exists: true,
                    fingerprint: Some("baseline".to_owned()),
                    text_snapshot: Some("b=1".to_owned()),
                },
            ),
        ]);

        let merged = daemon::merge_reloaded_observed(&previous, current);
        assert_eq!(
            merged
                .get("/tmp/one.conf")
                .and_then(|entry| entry.fingerprint.as_deref()),
            Some("old")
        );
        assert_eq!(
            merged
                .get("/tmp/two.conf")
                .and_then(|entry| entry.fingerprint.as_deref()),
            Some("baseline")
        );
    }

    #[test]
    fn merge_reloaded_observed_rebases_when_diff_mode_changes() {
        let previous = BTreeMap::from([(
            "/tmp/one.conf".to_owned(),
            daemon::ObservedPath {
                scope: Scope::System,
                path: "/tmp/one.conf".to_owned(),
                category: Category::Shell,
                diff_mode: DiffMode::MetadataOnly,
                redaction: RedactionMode::Auto,
                exists: true,
                fingerprint: Some("old".to_owned()),
                text_snapshot: None,
            },
        )]);

        let current = BTreeMap::from([(
            "/tmp/one.conf".to_owned(),
            daemon::ObservedPath {
                scope: Scope::System,
                path: "/tmp/one.conf".to_owned(),
                category: Category::Shell,
                diff_mode: DiffMode::Unified,
                redaction: RedactionMode::Auto,
                exists: true,
                fingerprint: Some("new".to_owned()),
                text_snapshot: Some("line=1".to_owned()),
            },
        )]);

        let merged = daemon::merge_reloaded_observed(&previous, current);
        assert_eq!(
            merged
                .get("/tmp/one.conf")
                .and_then(|entry| entry.fingerprint.as_deref()),
            Some("new")
        );
        assert_eq!(
            merged
                .get("/tmp/one.conf")
                .and_then(|entry| entry.text_snapshot.as_deref()),
            Some("line=1")
        );
    }

    #[test]
    fn startup_scan_baselines_newly_tracked_paths_without_created_events() {
        let previous = BTreeMap::from([(
            "/home/test/.config/fish/config.fish".to_owned(),
            daemon::ObservedPath {
                scope: Scope::User,
                path: "/home/test/.config/fish/config.fish".to_owned(),
                category: Category::Shell,
                diff_mode: DiffMode::MetadataOnly,
                redaction: RedactionMode::Auto,
                exists: true,
                fingerprint: Some("fish".to_owned()),
                text_snapshot: None,
            },
        )]);

        let current = BTreeMap::from([
            (
                "/home/test/.config/fish/config.fish".to_owned(),
                daemon::ObservedPath {
                    scope: Scope::User,
                    path: "/home/test/.config/fish/config.fish".to_owned(),
                    category: Category::Shell,
                    diff_mode: DiffMode::MetadataOnly,
                    redaction: RedactionMode::Auto,
                    exists: true,
                    fingerprint: Some("fish".to_owned()),
                    text_snapshot: None,
                },
            ),
            (
                "/home/test/.bashrc".to_owned(),
                daemon::ObservedPath {
                    scope: Scope::User,
                    path: "/home/test/.bashrc".to_owned(),
                    category: Category::Shell,
                    diff_mode: DiffMode::MetadataOnly,
                    redaction: RedactionMode::Auto,
                    exists: true,
                    fingerprint: Some("bash".to_owned()),
                    text_snapshot: None,
                },
            ),
        ]);

        let config = Config {
            version: 1,
            retention: RetentionPolicy::default(),
            tracked_paths: vec![
                TrackedPath {
                    path: "/home/test/.config/fish/config.fish".to_owned(),
                    category: Category::Shell,
                    kind: PathKind::File,
                    diff_mode: DiffMode::MetadataOnly,
                    redaction: RedactionMode::Auto,
                    source: TrackSource::Preset,
                },
                TrackedPath {
                    path: "/home/test/.bashrc".to_owned(),
                    category: Category::Shell,
                    kind: PathKind::File,
                    diff_mode: DiffMode::MetadataOnly,
                    redaction: RedactionMode::Auto,
                    source: TrackSource::Preset,
                },
            ],
            tracked_packages: Vec::new(),
        };

        let events = daemon::diff_startup_observed(&previous, &current, &config);
        assert!(events.is_empty());
    }
}
