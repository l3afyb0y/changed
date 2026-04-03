use crate::category::Category;
use crate::config::{Config, DiffMode, PathKind, RedactionMode, RetentionPolicy, TrackSource, TrackedPackage, TrackedPath};
use crate::journal::{EventKind, JournalEvent};
use anyhow::{anyhow, Context, Result};
use directories::BaseDirs;
use notify::{recommended_watcher, RecommendedWatcher, RecursiveMode, Watcher};
use regex::Regex;
use serde::{Deserialize, Serialize};
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, SystemTime};
use time::format_description::well_known::Rfc3339;
use time::{Date, OffsetDateTime, Time};
use walkdir::WalkDir;

const APP_NAME: &str = "changed";
const CONFIG_ENV: &str = "CHANGED_CONFIG_HOME";
const STATE_ENV: &str = "CHANGED_STATE_HOME";
const MAX_DIFF_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub config_home: PathBuf,
    pub state_home: PathBuf,
}

impl AppPaths {
    pub fn detect() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("failed to detect base directories")?;

        let config_home = env::var_os(CONFIG_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| base_dirs.config_dir().join(APP_NAME));
        let state_home = env::var_os(STATE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                base_dirs
                    .state_dir()
                    .unwrap_or_else(|| base_dirs.home_dir())
                    .join(".local/state")
                    .join(APP_NAME)
            });

        Ok(Self {
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

#[derive(Clone, Debug)]
pub struct App {
    pub paths: AppPaths,
}

#[derive(Clone, Debug)]
pub struct DaemonOptions {
    pub once: bool,
    pub interval: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct ObservedPath {
    path: String,
    category: Category,
    diff_mode: DiffMode,
    redaction: RedactionMode,
    exists: bool,
    fingerprint: Option<String>,
    text_snapshot: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct DaemonState {
    observed: BTreeMap<String, ObservedPath>,
}

impl App {
    pub fn new() -> Result<Self> {
        Ok(Self {
            paths: AppPaths::detect()?,
        })
    }

    pub fn init(&self) -> Result<String> {
        fs::create_dir_all(&self.paths.config_home).context("failed to create config directory")?;
        fs::create_dir_all(&self.paths.state_home).context("failed to create state directory")?;

        let config_path = self.paths.config_file();
        if config_path.exists() {
            let config = self.load_config()?;
            return Ok(render_init_summary(&self.paths, &config, false));
        }

        let mut config = Config::new();
        config.tracked_paths = detect_presets()?;
        config.sort_and_dedup();
        self.save_config(&config)?;

        Ok(render_init_summary(&self.paths, &config, true))
    }

    pub fn list_tracked(&self, category: Option<Category>, path: Option<&Path>) -> Result<String> {
        let config = self.load_or_default()?;
        Ok(render_tracked(&config, category, path))
    }

    pub fn list_history(
        &self,
        category: Option<Category>,
        path: Option<&Path>,
        all: bool,
        since: Option<&str>,
        until: Option<&str>,
        clean: bool,
    ) -> Result<String> {
        let journal_exists = self.paths.journal_file().exists();
        let since = parse_filter_time(since)?;
        let until = parse_filter_time(until)?;
        let limit = if all { None } else { Some(50) };
        let events = self.load_filtered_events(category, path, since, until, limit)?;
        if events.is_empty() {
            return Ok(if journal_exists {
                String::from("No change history matched that filter.")
            } else {
                String::from("No change history recorded yet.")
            });
        }
        Ok(render_history(&events, clean, None))
    }

    pub fn run_daemon(&self, options: DaemonOptions) -> Result<String> {
        fs::create_dir_all(&self.paths.state_home).context("failed to create state directory")?;
        let mut config = self.load_or_default()?;
        if config.tracked_paths.is_empty() {
            return Ok(String::from(
                "Nothing is currently tracked. Add paths or categories before starting the daemon.",
            ));
        }

        let had_existing_state = self.paths.daemon_state_file().exists();
        let mut state = self.load_daemon_state()?;
        let mut runs = 0usize;
        let mut watcher = if options.once {
            None
        } else {
            Some(build_watcher(&config, &self.paths)?)
        };

        loop {
            if runs > 0 {
                match self.load_or_default() {
                    Ok(latest) if latest != config => {
                        config = latest;
                        let reloaded_observed = observe_tracked_paths(&config)?;
                        state.observed = merge_reloaded_observed(&state.observed, reloaded_observed);
                        watcher = if options.once {
                            None
                        } else {
                            Some(build_watcher(&config, &self.paths)?)
                        };
                        println!("Reloaded config. Watching {} tracked target(s)...", config.tracked_paths.len());
                    }
                    Ok(_) => {}
                    Err(err) => {
                        eprintln!("Failed to reload config; keeping previous configuration: {err}");
                    }
                }
            }

            let observed = observe_tracked_paths(&config)?;
            let events = if had_existing_state || !state.observed.is_empty() {
                diff_observed(&state.observed, &observed)
            } else {
                Vec::new()
            };
            if !events.is_empty() {
                self.append_events(&events)?;
                self.enforce_journal_retention(&config.retention)?;
            }
            state.observed = observed;
            self.save_daemon_state(&state)?;

            runs += 1;
            if options.once {
                if !had_existing_state && runs == 1 {
                    return Ok(String::from("Baseline captured. Recorded 0 change events."));
                }
                return Ok(format!(
                    "Daemon scan complete. Recorded {} change event{}.",
                    events.len(),
                    pluralize(events.len())
                ));
            }

            if runs == 1 && !had_existing_state {
                println!("Baseline captured. Watching {} tracked target(s)...", config.tracked_paths.len());
            } else if !events.is_empty() {
                println!("Recorded {} change event{}.", events.len(), pluralize(events.len()));
            }
            wait_for_next_scan(watcher.as_mut(), options.interval);
        }
    }

    pub fn track_file(&self, raw_path: &str) -> Result<String> {
        let mut config = self.load_or_default()?;
        let expanded = expand_user_path(raw_path)?;
        let kind = detect_path_kind(&expanded);
        let path = normalize_display_path(&expanded);
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
        self.save_config(&config)?;

        Ok(format!(
            "Now tracking {} under '{}' ({}, redaction {}).",
            path,
            category,
            diff_mode_label(diff_mode),
            redaction_label(redaction)
        ))
    }

    pub fn track_category(&self, category: Category) -> Result<String> {
        let mut config = self.load_or_default()?;
        let preset_targets = preset_targets_for_category(category)?;
        if preset_targets.is_empty() {
            return Ok(format!(
                "No matching preset paths were found for '{}' on this system.",
                category
            ));
        }

        let count_before = config.tracked_paths.len();
        for target in preset_targets {
            upsert_path(&mut config, target);
        }
        self.save_config(&config)?;

        let added = config.tracked_paths.len().saturating_sub(count_before);
        Ok(format!(
            "Tracked category '{}'. Added {} preset target{}.",
            category,
            added,
            pluralize(added)
        ))
    }

    pub fn track_package(&self, manager: &str, package_name: &str) -> Result<String> {
        let mut config = self.load_or_default()?;
        config.tracked_packages.push(TrackedPackage {
            manager: manager.to_owned(),
            package_name: package_name.to_owned(),
            source: TrackSource::Manual,
        });
        config.sort_and_dedup();
        self.save_config(&config)?;

        Ok(format!("Now tracking package target: {} {}", manager, package_name))
    }

    pub fn untrack_file(&self, raw_path: &str) -> Result<String> {
        let mut config = self.load_or_default()?;
        let path = normalize_display_path(&expand_user_path(raw_path)?);
        let before = config.tracked_paths.len();
        config.tracked_paths.retain(|entry| entry.path != path);
        self.save_config(&config)?;

        let removed = before.saturating_sub(config.tracked_paths.len());
        Ok(format!(
            "Stopped tracking {}. Removed {} target{}.",
            path,
            removed,
            pluralize(removed)
        ))
    }

    pub fn untrack_category(&self, category: Category) -> Result<String> {
        let mut config = self.load_or_default()?;
        let before = config.tracked_paths.len();
        config.tracked_paths.retain(|entry| entry.category != category);
        self.save_config(&config)?;

        let removed = before.saturating_sub(config.tracked_paths.len());
        Ok(format!(
            "Stopped tracking category '{}'. Removed {} target{}.",
            category,
            removed,
            pluralize(removed)
        ))
    }

    pub fn untrack_package(&self, manager: &str, package_name: &str) -> Result<String> {
        let mut config = self.load_or_default()?;
        let before = config.tracked_packages.len();
        config
            .tracked_packages
            .retain(|pkg| !(pkg.manager == manager && pkg.package_name == package_name));
        self.save_config(&config)?;

        let removed = before.saturating_sub(config.tracked_packages.len());
        Ok(format!(
            "Stopped tracking package target: {} {}. Removed {} record{}.",
            manager,
            package_name,
            removed,
            pluralize(removed)
        ))
    }

    pub fn set_diff_mode(&self, raw_path: &str, diff_mode: DiffMode) -> Result<String> {
        let mut config = self.load_or_default()?;
        let path = normalize_display_path(&expand_user_path(raw_path)?);
        let updated = set_path_policy(&mut config, &path, |entry| entry.diff_mode = diff_mode);
        self.save_config(&config)?;
        if updated {
            Ok(format!(
                "Updated diff mode for {} to {}.",
                path,
                diff_mode_label(diff_mode)
            ))
        } else {
            Err(anyhow!("{} is not tracked yet", path))
        }
    }

    pub fn set_redaction_mode(&self, raw_path: &str, redaction: RedactionMode) -> Result<String> {
        let mut config = self.load_or_default()?;
        let path = normalize_display_path(&expand_user_path(raw_path)?);
        let updated = set_path_policy(&mut config, &path, |entry| entry.redaction = redaction);
        self.save_config(&config)?;
        if updated {
            Ok(format!(
                "Updated redaction mode for {} to {}.",
                path,
                redaction_label(redaction)
            ))
        } else {
            Err(anyhow!("{} is not tracked yet", path))
        }
    }

    pub fn service_message(&self, action: &str) -> String {
        match action {
            "install" => String::from("Service installation is not implemented yet."),
            "start" => String::from("Service start is not implemented yet."),
            "stop" => String::from("Service stop is not implemented yet."),
            "status" => String::from("Service status is not implemented yet."),
            _ => String::from("Unknown service action."),
        }
    }

    fn load_or_default(&self) -> Result<Config> {
        let config_path = self.paths.config_file();
        if config_path.exists() {
            self.load_config()
        } else {
            Ok(Config::new())
        }
    }

    fn load_config(&self) -> Result<Config> {
        let path = self.paths.config_file();
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

    fn save_config(&self, config: &Config) -> Result<()> {
        fs::create_dir_all(&self.paths.config_home).context("failed to create config directory")?;
        let path = self.paths.config_file();
        let contents = toml::to_string_pretty(config).context("failed to serialize config")?;
        let temp_path = path.with_extension(format!(
            "toml.tmp.{}",
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(&temp_path, contents)
            .with_context(|| format!("failed to write temporary config file {}", temp_path.display()))?;
        fs::rename(&temp_path, &path).with_context(|| {
            format!(
                "failed to replace config file {} with {}",
                path.display(),
                temp_path.display()
            )
        })?;
        Ok(())
    }

    fn load_daemon_state(&self) -> Result<DaemonState> {
        let path = self.paths.daemon_state_file();
        if !path.exists() {
            return Ok(DaemonState::default());
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read daemon state {}", path.display()))?;
        let state = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse daemon state {}", path.display()))?;
        Ok(state)
    }

    fn save_daemon_state(&self, state: &DaemonState) -> Result<()> {
        fs::create_dir_all(&self.paths.state_home).context("failed to create state directory")?;
        let path = self.paths.daemon_state_file();
        let contents = serde_json::to_string_pretty(state).context("failed to serialize daemon state")?;
        let temp_path = path.with_extension(format!(
            "json.tmp.{}",
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::write(&temp_path, contents).with_context(|| {
            format!(
                "failed to write temporary daemon state {}",
                temp_path.display()
            )
        })?;
        fs::rename(&temp_path, &path).with_context(|| {
            format!(
                "failed to replace daemon state file {} with {}",
                path.display(),
                temp_path.display()
            )
        })?;
        Ok(())
    }

    fn append_events(&self, events: &[JournalEvent]) -> Result<()> {
        fs::create_dir_all(&self.paths.state_home).context("failed to create state directory")?;
        let path = self.paths.journal_file();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open journal {}", path.display()))?;
        for event in events {
            let line = serde_json::to_string(event).context("failed to serialize journal event")?;
            file.write_all(line.as_bytes()).context("failed to write journal event")?;
            file.write_all(b"\n").context("failed to write journal newline")?;
        }
        Ok(())
    }

    fn load_events(&self) -> Result<Vec<JournalEvent>> {
        let path = self.paths.journal_file();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        let file = fs::File::open(&path)
            .with_context(|| format!("failed to read journal {}", path.display()))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.with_context(|| format!("failed to read journal line in {}", path.display()))?;
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
        category: Option<Category>,
        path: Option<&Path>,
        since: Option<OffsetDateTime>,
        until: Option<OffsetDateTime>,
        limit: Option<usize>,
    ) -> Result<Vec<JournalEvent>> {
        let journal_path = self.paths.journal_file();
        if !journal_path.exists() {
            return Ok(Vec::new());
        }

        let path_filter = path.map(normalize_display_path);
        let file = fs::File::open(&journal_path)
            .with_context(|| format!("failed to read journal {}", journal_path.display()))?;
        let reader = BufReader::new(file);

        let mut limited = limit.map(|_| VecDeque::new());
        let mut all_events = if limit.is_none() { Some(Vec::new()) } else { None };

        for line in reader.lines() {
            let line = line.with_context(|| {
                format!("failed to read journal line in {}", journal_path.display())
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let event: JournalEvent = serde_json::from_str(&line)
                .with_context(|| format!("failed to parse journal line in {}", journal_path.display()))?;
            if !event_matches_filters(&event, category, path_filter.as_deref(), since, until) {
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

    fn enforce_journal_retention(&self, retention: &RetentionPolicy) -> Result<()> {
        let path = self.paths.journal_file();
        if !path.exists() {
            return Ok(());
        }

        let metadata = fs::metadata(&path)
            .with_context(|| format!("failed to read journal metadata {}", path.display()))?;
        let events = self.load_events()?;
        let exceeds_events = events.len() > retention.max_events;
        let exceeds_bytes = metadata.len() > retention.max_bytes;
        if !exceeds_events && !exceeds_bytes {
            return Ok(());
        }

        let start_index = retention_start_index(&events, retention);
        let retained = &events[start_index..];
        let mut file = fs::File::create(&path)
            .with_context(|| format!("failed to truncate journal {}", path.display()))?;
        for event in retained {
            let line = serde_json::to_string(event).context("failed to serialize retained journal event")?;
            file.write_all(line.as_bytes())
                .context("failed to write retained journal event")?;
            file.write_all(b"\n")
                .context("failed to write retained journal newline")?;
        }
        Ok(())
    }
}

fn render_init_summary(paths: &AppPaths, config: &Config, created: bool) -> String {
    let mut out = String::new();
    if created {
        out.push_str("Initialized changed.\n");
    } else {
        out.push_str("changed is already initialized.\n");
    }
    let _ = writeln!(out, "Config: {}", paths.config_file().display());
    let _ = writeln!(out, "State: {}", paths.state_home.display());
    let _ = writeln!(
        out,
        "Tracked paths: {} | tracked packages: {}",
        config.tracked_paths.len(),
        config.tracked_packages.len()
    );

    if !config.tracked_paths.is_empty() {
        out.push_str("Enabled categories:\n");
        for category in Category::ALL {
            let count = config
                .tracked_paths
                .iter()
                .filter(|entry| entry.category == category)
                .count();
            if count > 0 {
                let _ = writeln!(out, "  - {} ({})", category, count);
            }
        }
    }

    out
}

fn render_tracked(config: &Config, category: Option<Category>, path: Option<&Path>) -> String {
    let mut out = String::new();
    let path_filter = path.map(normalize_display_path);

    let filtered_paths: Vec<&TrackedPath> = config
        .tracked_paths
        .iter()
        .filter(|entry| category.is_none_or(|wanted| entry.category == wanted))
        .filter(|entry| {
            path_filter
                .as_ref()
                .is_none_or(|wanted| entry.path.as_str() == wanted.as_str())
        })
        .collect();

    let filtered_packages: Vec<&TrackedPackage> = config
        .tracked_packages
        .iter()
        .filter(|_| category.is_none_or(|wanted| wanted == Category::Packages))
        .collect();

    if filtered_paths.is_empty() && filtered_packages.is_empty() {
        return String::from("Nothing is currently tracked for that filter.");
    }

    for current in Category::ALL {
        if category.is_some_and(|wanted| wanted != current) {
            continue;
        }

        let section_paths: Vec<&TrackedPath> = filtered_paths
            .iter()
            .copied()
            .filter(|entry| entry.category == current)
            .collect();
        let section_packages: Vec<&TrackedPackage> = filtered_packages
            .iter()
            .copied()
            .filter(|_| current == Category::Packages)
            .collect();

        if section_paths.is_empty() && section_packages.is_empty() {
            continue;
        }

        let _ = writeln!(out, "{}:", current);
        for entry in section_paths {
            let _ = writeln!(
                out,
                "  - {} [{}; {}; {}]",
                entry.path,
                kind_label(entry.kind),
                diff_mode_label(entry.diff_mode),
                redaction_label(entry.redaction)
            );
        }
        for pkg in section_packages {
            let _ = writeln!(out, "  - {} {}", pkg.manager, pkg.package_name);
        }
    }

    out.trim_end().to_owned()
}

fn render_history(events: &[JournalEvent], clean: bool, limit: Option<usize>) -> String {
    let mut sorted: Vec<&JournalEvent> = events.iter().collect();
    sorted.sort_by_key(|event| event.timestamp);
    let selected: Vec<&JournalEvent> = match limit {
        Some(limit) => sorted.into_iter().rev().take(limit).collect::<Vec<_>>().into_iter().rev().collect(),
        None => sorted,
    };

    let mut out = String::from("# Changes\n\n");
    let mut current_date: Option<Date> = None;
    for event in selected {
        let date = event.timestamp.date();
        if current_date != Some(date) {
            if current_date.is_some() {
                out.push('\n');
            }
            current_date = Some(date);
            let _ = writeln!(out, "## {}\n", format_date(date));
        }

        if clean {
            let _ = writeln!(
                out,
                "- {} [{}] {}",
                format_time(event.timestamp.time()),
                event.category,
                clean_summary(event)
            );
        } else {
            let _ = writeln!(out, "### {}", format_time(event.timestamp.time()));
            let _ = writeln!(out, "{}", event.path);
            let _ = writeln!(out, "{}", event.summary);
            if let Some(diff) = &event.diff {
                for line in diff.lines() {
                    let _ = writeln!(out, "{line}");
                }
            }
            out.push('\n');
        }
    }

    out.trim_end().to_owned()
}

fn observe_tracked_paths(config: &Config) -> Result<BTreeMap<String, ObservedPath>> {
    let mut observed = BTreeMap::new();
    for tracked in &config.tracked_paths {
        for candidate in expand_tracked_target(tracked)? {
            observed.insert(candidate.path.clone(), candidate);
        }
    }
    Ok(observed)
}

fn expand_tracked_target(tracked: &TrackedPath) -> Result<Vec<ObservedPath>> {
    let path = PathBuf::from(&tracked.path);
    match tracked.kind {
        PathKind::File => Ok(vec![observe_single_path(
            path,
            tracked.category,
            tracked.diff_mode,
            tracked.redaction,
        )?]),
        PathKind::Directory => {
            if !path.exists() {
                return Ok(vec![observe_single_path(
                    path,
                    tracked.category,
                    tracked.diff_mode,
                    tracked.redaction,
                )?]);
            }

            let mut entries = Vec::new();
            for entry in WalkDir::new(&path).into_iter().filter_map(|entry| entry.ok()) {
                if entry.file_type().is_dir() {
                    continue;
                }
                entries.push(observe_single_path(
                    entry.path().to_path_buf(),
                    tracked.category,
                    tracked.diff_mode,
                    tracked.redaction,
                )?);
            }
            if entries.is_empty() {
                entries.push(observe_single_path(
                    path,
                    tracked.category,
                    tracked.diff_mode,
                    tracked.redaction,
                )?);
            }
            Ok(entries)
        }
    }
}

fn observe_single_path(
    path: PathBuf,
    category: Category,
    diff_mode: DiffMode,
    redaction: RedactionMode,
) -> Result<ObservedPath> {
    let display = normalize_display_path(&path);
    if !path.exists() {
        return Ok(ObservedPath {
            path: display,
            category,
            diff_mode,
            redaction,
            exists: false,
            fingerprint: None,
            text_snapshot: None,
        });
    }

    let metadata = fs::metadata(&path)
        .with_context(|| format!("failed to read metadata for {}", path.display()))?;
    let modified = metadata.modified().ok().and_then(system_time_secs);
    if metadata.is_dir() {
        let mut hasher = blake3::Hasher::new();
        hasher.update(display.as_bytes());
        hasher.update(format!("{modified:?}").as_bytes());
        return Ok(ObservedPath {
            path: display,
            category,
            diff_mode,
            redaction,
            exists: true,
            fingerprint: Some(hasher.finalize().to_hex().to_string()),
            text_snapshot: None,
        });
    }

    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let fingerprint = blake3::hash(&bytes).to_hex().to_string();
    let text_snapshot = if diff_mode == DiffMode::Unified && metadata.len() <= MAX_DIFF_BYTES {
        String::from_utf8(bytes.clone())
            .ok()
            .map(|text| maybe_redact_text(text, redaction))
    } else {
        None
    };

    Ok(ObservedPath {
        path: display,
        category,
        diff_mode,
        redaction,
        exists: true,
        fingerprint: Some(fingerprint),
        text_snapshot,
    })
}

fn diff_observed(
    previous: &BTreeMap<String, ObservedPath>,
    current: &BTreeMap<String, ObservedPath>,
) -> Vec<JournalEvent> {
    let mut keys = BTreeSet::new();
    keys.extend(previous.keys().cloned());
    keys.extend(current.keys().cloned());

    let mut events = Vec::new();
    for key in keys {
        let before = previous.get(&key);
        let after = current.get(&key);
        match (before, after) {
            (None, Some(after)) if after.exists => events.push(build_event(EventKind::Created, None, Some(after))),
            (Some(before), None) if before.exists => events.push(build_event(EventKind::Removed, Some(before), None)),
            (Some(before), Some(after)) => {
                if before.exists != after.exists {
                    if after.exists {
                        events.push(build_event(EventKind::Created, Some(before), Some(after)));
                    } else {
                        events.push(build_event(EventKind::Removed, Some(before), Some(after)));
                    }
                } else if before.fingerprint != after.fingerprint {
                    events.push(build_event(EventKind::Modified, Some(before), Some(after)));
                }
            }
            _ => {}
        }
    }
    events
}

fn build_event(
    kind: EventKind,
    before: Option<&ObservedPath>,
    after: Option<&ObservedPath>,
) -> JournalEvent {
    let reference = after.or(before).expect("an event requires a reference path");
    let diff = match kind {
        EventKind::Modified | EventKind::Created => build_diff(before, after),
        EventKind::Removed => None,
    };
    let (added_lines, removed_lines) = diff_line_counts(diff.as_deref());
    JournalEvent {
        timestamp: OffsetDateTime::now_utc(),
        kind: kind.clone(),
        category: reference.category,
        path: reference.path.clone(),
        summary: summarize_event(&kind, reference.category, added_lines, removed_lines),
        added_lines,
        removed_lines,
        diff,
    }
}

fn build_diff(before: Option<&ObservedPath>, after: Option<&ObservedPath>) -> Option<String> {
    let old = before.and_then(|entry| entry.text_snapshot.as_deref()).unwrap_or("");
    let new = after.and_then(|entry| entry.text_snapshot.as_deref()).unwrap_or("");
    if old.is_empty() && new.is_empty() {
        return None;
    }

    let diff = TextDiff::from_lines(old, new);
    let mut lines = Vec::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Delete => lines.push(format!("(-) {}", change)),
            similar::ChangeTag::Insert => lines.push(format!("(+) {}", change)),
            similar::ChangeTag::Equal => {}
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(
            lines
                .into_iter()
                .map(|line| line.trim_end_matches('\n').to_owned())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

fn summarize_event(kind: &EventKind, category: Category, added_lines: usize, removed_lines: usize) -> String {
    let area = category_label(category);
    match kind {
        EventKind::Created => {
            if added_lines > 0 {
                format!("Created {area} (+{added_lines})")
            } else {
                format!("Created {area}")
            }
        }
        EventKind::Modified => match (added_lines, removed_lines) {
            (0, 0) => format!("Changed {area}"),
            (adds, 0) => format!("Changed {area} (+{adds})"),
            (0, removes) => format!("Changed {area} (-{removes})"),
            (adds, removes) => format!("Changed {area} (+{adds}/-{removes})"),
        },
        EventKind::Removed => format!("Removed {area}"),
    }
}

fn clean_summary(event: &JournalEvent) -> String {
    let mut line = format!("{}: {}", event.path, event.summary);
    if event.diff.is_none() && event.kind == EventKind::Modified {
        line.push_str(" [metadata-only]");
    }
    line
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::Cpu => "CPU tuning",
        Category::Gpu => "GPU tuning",
        Category::Services => "service config",
        Category::Scheduler => "scheduler tuning",
        Category::Shell => "shell config",
        Category::Build => "build config",
        Category::Boot => "boot config",
        Category::Audio => "audio config",
        Category::Packages => "package tracking",
    }
}

fn diff_line_counts(diff: Option<&str>) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    if let Some(diff) = diff {
        for line in diff.lines() {
            if line.starts_with("(+) ") {
                added += 1;
            } else if line.starts_with("(-) ") {
                removed += 1;
            }
        }
    }
    (added, removed)
}

fn merge_reloaded_observed(
    previous: &BTreeMap<String, ObservedPath>,
    current: BTreeMap<String, ObservedPath>,
) -> BTreeMap<String, ObservedPath> {
    let mut merged = BTreeMap::new();
    for (path, current_observed) in current {
        if let Some(previous_observed) = previous.get(&path) {
            merged.insert(path, previous_observed.clone());
        } else {
            merged.insert(path, current_observed);
        }
    }
    merged
}

fn event_matches_filters(
    event: &JournalEvent,
    category: Option<Category>,
    path: Option<&str>,
    since: Option<OffsetDateTime>,
    until: Option<OffsetDateTime>,
) -> bool {
    category.is_none_or(|wanted| event.category == wanted)
        && path.is_none_or(|wanted| wanted == event.path)
        && since.is_none_or(|value| event.timestamp >= value)
        && until.is_none_or(|value| event.timestamp <= value)
}

fn retention_start_index(events: &[JournalEvent], retention: &RetentionPolicy) -> usize {
    if events.is_empty() {
        return 0;
    }

    let mut start = events.len().saturating_sub(retention.max_events);
    let mut encoded_size = 0u64;
    for event in events.iter().skip(start).rev() {
        let line_size = serde_json::to_string(event)
            .map(|line| line.len() as u64 + 1)
            .unwrap_or(0);
        if encoded_size + line_size > retention.max_bytes {
            break;
        }
        encoded_size += line_size;
        start = start.saturating_sub(1);
    }

    while start < events.len() {
        let retained = &events[start..];
        let count_ok = retained.len() <= retention.max_events;
        let bytes_ok = retained
            .iter()
            .try_fold(0u64, |acc, event| {
                serde_json::to_string(event)
                    .map(|line| acc + line.len() as u64 + 1)
                    .ok()
            })
            .is_some_and(|size| size <= retention.max_bytes);
        if count_ok && bytes_ok {
            break;
        }
        start += 1;
    }

    start.min(events.len())
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

fn format_date(date: Date) -> String {
    format!("{:02}/{:02}/{:02}", u8::from(date.month()), date.day(), date.year() % 100)
}

fn format_time(time: Time) -> String {
    let hour = time.hour();
    let period = if hour >= 12 { "pm" } else { "am" };
    let mut display_hour = hour % 12;
    if display_hour == 0 {
        display_hour = 12;
    }
    format!("{display_hour}:{:02}{period}", time.minute())
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

fn detect_presets() -> Result<Vec<TrackedPath>> {
    let mut all = Vec::new();
    for category in Category::ALL {
        if category == Category::Packages {
            continue;
        }
        all.extend(preset_targets_for_category(category)?);
    }
    Ok(all)
}

fn preset_targets_for_category(category: Category) -> Result<Vec<TrackedPath>> {
    let home = home_dir()?;
    let profile = HardwareProfile::detect();

    let candidates = match category {
        Category::Cpu => vec![
            Some(preset_file("/etc/default/cpupower", category, DiffMode::Unified, RedactionMode::Off)),
            profile
                .cpu_vendor
                .filter(|vendor| *vendor == "amd")
                .map(|_| preset_file("/etc/modprobe.d/amd-pstate.conf", category, DiffMode::Unified, RedactionMode::Off)),
            profile
                .cpu_vendor
                .filter(|vendor| *vendor == "intel")
                .map(|_| preset_file("/etc/modprobe.d/intel-pstate.conf", category, DiffMode::Unified, RedactionMode::Off)),
        ],
        Category::Gpu => vec![
            profile
                .gpu_vendor
                .filter(|vendor| *vendor == "nvidia")
                .map(|_| preset_file("/etc/modprobe.d/nvidia.conf", category, DiffMode::Unified, RedactionMode::Off)),
            profile
                .gpu_vendor
                .filter(|vendor| *vendor == "amd")
                .map(|_| preset_file("/etc/X11/xorg.conf.d/20-amdgpu.conf", category, DiffMode::Unified, RedactionMode::Off)),
        ],
        Category::Services => vec![
            Some(preset_file("/etc/systemd/system.conf", category, DiffMode::Unified, RedactionMode::Off)),
            Some(preset_dir(home.join(".config/systemd/user"), category, DiffMode::MetadataOnly, RedactionMode::Off)),
        ],
        Category::Scheduler => vec![
            Some(preset_file("/etc/sysctl.d/99-scheduler.conf", category, DiffMode::Unified, RedactionMode::Off)),
            Some(preset_file("/etc/udev/rules.d/60-ioschedulers.rules", category, DiffMode::Unified, RedactionMode::Off)),
        ],
        Category::Shell => vec![
            Some(preset_file(home.join(".config/fish/config.fish"), category, DiffMode::MetadataOnly, RedactionMode::Auto)),
            Some(preset_file(home.join(".bashrc"), category, DiffMode::MetadataOnly, RedactionMode::Auto)),
            Some(preset_file(home.join(".zshrc"), category, DiffMode::MetadataOnly, RedactionMode::Auto)),
        ],
        Category::Build => vec![
            Some(preset_file("/etc/makepkg.conf", category, DiffMode::Unified, RedactionMode::Off)),
            Some(preset_file(home.join(".config/pacman/makepkg.conf"), category, DiffMode::Unified, RedactionMode::Off)),
        ],
        Category::Boot => vec![
            Some(preset_file("/etc/default/grub", category, DiffMode::Unified, RedactionMode::Off)),
            Some(preset_file("/etc/mkinitcpio.conf", category, DiffMode::Unified, RedactionMode::Off)),
            Some(preset_dir("/boot/loader/entries", category, DiffMode::MetadataOnly, RedactionMode::Off)),
        ],
        Category::Audio => vec![
            Some(preset_file(home.join(".config/pipewire/pipewire.conf"), category, DiffMode::Unified, RedactionMode::Off)),
            Some(preset_dir(home.join(".config/wireplumber/wireplumber.conf.d"), category, DiffMode::MetadataOnly, RedactionMode::Off)),
            Some(preset_file("/etc/pipewire/pipewire.conf", category, DiffMode::Unified, RedactionMode::Off)),
        ],
        Category::Packages => Vec::new(),
    };

    Ok(candidates
        .into_iter()
        .flatten()
        .filter(|entry| Path::new(&entry.path).exists())
        .collect())
}

fn preset_file<P: Into<PathBuf>>(
    path: P,
    category: Category,
    diff_mode: DiffMode,
    redaction: RedactionMode,
) -> TrackedPath {
    let path = path.into();
    TrackedPath {
        path: normalize_display_path(&path),
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
        path: normalize_display_path(&path),
        category,
        kind: PathKind::Directory,
        diff_mode,
        redaction,
        source: TrackSource::Preset,
    }
}

fn infer_category_for_path(path: &Path) -> Category {
    let path_str = normalize_display_path(path);
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

fn maybe_redact_text(text: String, redaction: RedactionMode) -> String {
    if redaction == RedactionMode::Off {
        return text;
    }

    let assignment = assignment_secret_regex();
    let export = export_secret_regex();
    let fish_set = fish_secret_regex();
    let url_auth = url_auth_regex();
    let sensitive_query = sensitive_query_regex();
    let bearer_token = bearer_token_regex();
    let auth_header = authorization_header_regex();
    let suspicious_literal = suspicious_literal_regex();
    let private_key_begin = private_key_begin_regex();
    let private_key_end = private_key_end_regex();

    let mut in_private_key_block = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if private_key_begin.is_match(line) {
            in_private_key_block = true;
            lines.push(String::from("[REDACTED PRIVATE KEY BLOCK]"));
            continue;
        }
        if in_private_key_block {
            if private_key_end.is_match(line) {
                in_private_key_block = false;
            }
            continue;
        }

        let redacted_line = if let Some(captures) = export.captures(line) {
            format!("{}{}{}[REDACTED]", &captures[1], &captures[2], &captures[4])
        } else if let Some(captures) = assignment.captures(line) {
            format!("{} = [REDACTED]", &captures[1])
        } else if let Some(captures) = fish_set.captures(line) {
            format!("{}{} [REDACTED]", &captures[1], &captures[2])
        } else if auth_header.is_match(line) {
            auth_header
                .replace_all(line, "${prefix}[REDACTED]")
                .into_owned()
        } else if bearer_token.is_match(line) {
            bearer_token
                .replace_all(line, "${prefix}[REDACTED]")
                .into_owned()
        } else if url_auth.is_match(line) {
            url_auth.replace_all(line, "${scheme}[REDACTED]@").into_owned()
        } else if sensitive_query.is_match(line) {
            sensitive_query
                .replace_all(line, "${prefix}${name}=[REDACTED]")
                .into_owned()
        } else if suspicious_literal.is_match(line) {
            suspicious_literal
                .replace_all(line, "${prefix}[REDACTED]")
                .into_owned()
        } else {
            line.to_owned()
        };
        lines.push(redacted_line);
    }

    if in_private_key_block {
        lines.push(String::from("[REDACTED PRIVATE KEY BLOCK]"));
    }

    lines.join("\n")
}

struct ActiveWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
}

fn build_watcher(config: &Config, paths: &AppPaths) -> Result<ActiveWatcher> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("failed to create filesystem watcher")?;

    for root in watch_roots(config, paths) {
        let recursive_mode = if root.is_dir() {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        watcher
            .watch(&root, recursive_mode)
            .with_context(|| format!("failed to watch {}", root.display()))?;
    }

    Ok(ActiveWatcher {
        _watcher: watcher,
        rx,
    })
}

fn wait_for_next_scan(watcher: Option<&mut ActiveWatcher>, interval: Duration) {
    let Some(watcher) = watcher else {
        thread::sleep(interval);
        return;
    };

    match watcher.rx.recv_timeout(interval) {
        Ok(Ok(_event)) => while watcher.rx.try_recv().is_ok() {},
        Ok(Err(err)) => eprintln!("Watcher error: {err}"),
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            eprintln!("Watcher channel disconnected; falling back to interval polling.");
            thread::sleep(interval);
        }
    }
}

fn watch_roots(config: &Config, paths: &AppPaths) -> Vec<PathBuf> {
    let mut roots = BTreeSet::new();
    roots.insert(paths.config_home.clone());
    for tracked in &config.tracked_paths {
        let path = PathBuf::from(&tracked.path);
        match tracked.kind {
            PathKind::Directory => {
                roots.insert(path);
            }
            PathKind::File => {
                if path.exists() {
                    roots.insert(path);
                } else if let Some(parent) = path.parent() {
                    roots.insert(parent.to_path_buf());
                }
            }
        }
    }
    roots.into_iter().collect()
}

fn assignment_secret_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*([A-Z0-9_]*(TOKEN|KEY|SECRET|PASSWORD|PASS|API)[A-Z0-9_]*)\s*=\s*(.+)$",
        )
            .expect("assignment regex should compile")
    })
}

fn export_secret_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)\b(export\s+)([A-Z0-9_]*(TOKEN|KEY|SECRET|PASSWORD|PASS|API)[A-Z0-9_]*)(\s*=\s*)(.+)",
        )
        .expect("export regex should compile")
    })
}

fn fish_secret_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)\b(set\s+-[a-zA-Z]*\s+)([A-Z0-9_]*(TOKEN|KEY|SECRET|PASSWORD|PASS|API)[A-Z0-9_]*)(?:\s+.+)?$",
        )
        .expect("fish secret regex should compile")
    })
}

fn url_auth_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?P<scheme>[a-zA-Z][a-zA-Z0-9+.-]*://)[^/@\s:]+:[^/@\s]+@")
            .expect("url auth regex should compile")
    })
}

fn sensitive_query_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<prefix>[?&])(?P<name>(token|key|secret|password|pass|session|auth|apikey))=([^&\s]+)",
        )
        .expect("sensitive query regex should compile")
    })
}

fn bearer_token_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)(?P<prefix>\bBearer\s+)([A-Za-z0-9._~+/=-]{8,})")
            .expect("bearer token regex should compile")
    })
}

fn authorization_header_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)(?P<prefix>Authorization\s*:\s*)(.+)")
            .expect("authorization header regex should compile")
    })
}

fn suspicious_literal_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)(?P<prefix>\b(client_secret|aws_secret_access_key|aws_access_key_id|github_token|gitlab_token|npm_token|auth_token)\b\s*[:=]\s*["']?)([^"'\s]+)"#,
        )
        .expect("suspicious literal regex should compile")
    })
}

fn private_key_begin_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----")
            .expect("private key begin regex should compile")
    })
}

fn private_key_end_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"-----END [A-Z0-9 ]*PRIVATE KEY-----")
            .expect("private key end regex should compile")
    })
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

fn kind_label(kind: PathKind) -> &'static str {
    match kind {
        PathKind::File => "file",
        PathKind::Directory => "dir",
    }
}

fn pluralize(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn detect_path_kind(path: &Path) -> PathKind {
    if path.is_dir() {
        PathKind::Directory
    } else {
        PathKind::File
    }
}

fn normalize_display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn expand_user_path(raw_path: &str) -> Result<PathBuf> {
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

fn home_dir() -> Result<PathBuf> {
    BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .context("failed to detect home directory")
}

fn system_time_secs(value: SystemTime) -> Option<u64> {
    value.duration_since(SystemTime::UNIX_EPOCH).ok().map(|duration| duration.as_secs())
}

#[derive(Default)]
struct HardwareProfile {
    cpu_vendor: Option<&'static str>,
    gpu_vendor: Option<&'static str>,
}

impl HardwareProfile {
    fn detect() -> Self {
        Self {
            cpu_vendor: detect_cpu_vendor(),
            gpu_vendor: detect_gpu_vendor(),
        }
    }
}

fn detect_cpu_vendor() -> Option<&'static str> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
    if cpuinfo.contains("AuthenticAMD") {
        Some("amd")
    } else if cpuinfo.contains("GenuineIntel") {
        Some("intel")
    } else {
        None
    }
}

fn detect_gpu_vendor() -> Option<&'static str> {
    if Path::new("/sys/module/nvidia").exists() {
        return Some("nvidia");
    }

    let vendor_file = "/sys/class/drm/card0/device/vendor";
    let vendor = fs::read_to_string(vendor_file).ok()?;
    match vendor.trim() {
        "0x1002" => Some("amd"),
        "0x10de" => Some("nvidia"),
        "0x8086" => Some("intel"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            let config_home = self.root.join("config");
            let state_home = self.root.join("state");
            App {
                paths: AppPaths {
                    config_home,
                    state_home,
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

        app.track_file("/etc/makepkg.conf").expect("tracking should succeed");
        let config = app.load_config().expect("config should load");

        assert_eq!(config.tracked_paths.len(), 1);
        assert_eq!(config.tracked_paths[0].category, Category::Build);
        assert_eq!(config.tracked_paths[0].diff_mode, DiffMode::Unified);
    }

    #[test]
    fn track_category_adds_matching_presets() {
        let env = TestEnv::new();
        let app = env.app();
        let fish_dir = home_dir().expect("home should exist").join(".config/fish");
        fs::create_dir_all(&fish_dir).expect("fish config dir should be creatable");
        let fish_config = fish_dir.join("config.fish");
        if !fish_config.exists() {
            fs::write(&fish_config, "# test\n").expect("fish config should be writable");
        }

        app.track_category(Category::Shell)
            .expect("category tracking should succeed");
        let config = app.load_config().expect("config should load");
        assert!(config
            .tracked_paths
            .iter()
            .any(|entry| entry.category == Category::Shell));
    }

    #[test]
    fn diff_update_requires_existing_path() {
        let env = TestEnv::new();
        let app = env.app();

        let error = app
            .set_diff_mode("/does/not/exist", DiffMode::Unified)
            .expect_err("missing tracked path should fail");
        assert!(error.to_string().contains("is not tracked yet"));
    }

    #[test]
    fn redaction_masks_sensitive_assignments() {
        let text = "export API_KEY=abc123\nset -gx SESSION_TOKEN xyz\nurl=https://user:pass@example.com\nrequest=Authorization: Bearer abcdefghijklmnop\ncallback=https://example.com?token=abc123&plain=value\nclient_secret = supersecret\n-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n-----END OPENSSH PRIVATE KEY-----\nPLAIN_VAR=value";
        let redacted = maybe_redact_text(text.to_owned(), RedactionMode::Auto);
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
    fn history_render_groups_by_date() {
        let events = vec![
            JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T01:00:00Z", &Rfc3339)
                    .expect("timestamp should parse"),
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
                kind: EventKind::Modified,
                category: Category::Shell,
                path: "/home/test/.config/fish/config.fish".to_owned(),
                summary: "Changed shell config".to_owned(),
                added_lines: 0,
                removed_lines: 0,
                diff: None,
            },
        ];

        let rendered = render_history(&events, false, None);
        assert!(rendered.contains("# Changes"));
        assert!(rendered.contains("## 04/03/26"));
        assert!(rendered.contains("## 04/04/26"));
    }

    #[test]
    fn clean_history_render_is_compact() {
        let events = vec![JournalEvent {
            timestamp: OffsetDateTime::parse("2026-04-03T01:00:00Z", &Rfc3339)
                .expect("timestamp should parse"),
            kind: EventKind::Modified,
            category: Category::Build,
            path: "/etc/makepkg.conf".to_owned(),
            summary: "Changed build config (+2/-1)".to_owned(),
            added_lines: 2,
            removed_lines: 1,
            diff: Some("(-) MAKEFLAGS=-j8\n(+) MAKEFLAGS=-j16".to_owned()),
        }];

        let rendered = render_history(&events, true, None);
        assert!(rendered.contains("- 1:00am [build] /etc/makepkg.conf: Changed build config (+2/-1)"));
        assert!(!rendered.contains("(+) MAKEFLAGS"));
    }

    #[test]
    fn first_daemon_scan_captures_baseline_without_events() {
        let env = TestEnv::new();
        let app = env.app();
        let tracked = env.root.join("tracked.conf");
        fs::write(&tracked, "value=1\n").expect("tracked file should be writable");

        app.track_file(tracked.to_string_lossy().as_ref())
            .expect("tracking should succeed");
        let message = app
            .run_daemon(DaemonOptions {
                once: true,
                interval: Duration::from_secs(1),
            })
            .expect("daemon run should succeed");

        assert!(message.contains("Baseline captured"));
        let events = app.load_events().expect("events should load");
        assert!(events.is_empty());
    }

    #[test]
    fn retention_start_index_respects_event_limit() {
        let events = (0..5)
            .map(|index| JournalEvent {
                timestamp: OffsetDateTime::parse("2026-04-03T01:00:00Z", &Rfc3339)
                    .expect("timestamp should parse")
                    + time::Duration::minutes(index),
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
        let start = retention_start_index(&events, &retention);
        assert_eq!(start, 3);
    }

    #[test]
    fn merge_reloaded_observed_preserves_existing_and_baselines_new() {
        let previous = BTreeMap::from([(
            "/tmp/one.conf".to_owned(),
            ObservedPath {
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
                ObservedPath {
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
                ObservedPath {
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

        let merged = merge_reloaded_observed(&previous, current);
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
}
