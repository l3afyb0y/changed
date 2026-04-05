use crate::category::Category;
use crate::config::{Config, DiffMode, PathKind, RedactionMode, RetentionPolicy, TrackedPath};
use crate::journal::{EventKind, JournalEvent};
use crate::scope::Scope;
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;
use time::OffsetDateTime;
use walkdir::WalkDir;

use super::paths::normalize_display_path;
use super::watch::{WatchPlan, WorkItem, build_watcher};
use super::{App, DaemonOptions};

const MAX_DIFF_BYTES: u64 = 256 * 1024;
const WORK_QUEUE_CAPACITY: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ObservedPath {
    pub(crate) scope: Scope,
    pub(crate) path: String,
    pub(crate) category: Category,
    pub(crate) diff_mode: DiffMode,
    pub(crate) redaction: RedactionMode,
    pub(crate) exists: bool,
    pub(crate) fingerprint: Option<String>,
    pub(crate) text_snapshot: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct DaemonState {
    pub observed: BTreeMap<String, ObservedPath>,
}

pub fn run(app: &App, scope: Scope, options: DaemonOptions) -> Result<String> {
    let paths = app.paths_for_scope(scope);
    super::ensure_scope_directories(paths)?;
    let mut config = app.load_or_default(scope)?;
    if config.tracked_paths.is_empty() {
        return Ok(String::from(
            "Nothing is currently tracked. Add paths or categories before starting the daemon.",
        ));
    }

    let had_existing_state = paths.daemon_state_file().exists();
    let mut state = app.load_daemon_state(scope)?;
    let initial_events = run_full_scan(app, scope, &config, &mut state, had_existing_state)?;

    if options.once {
        return if !had_existing_state && initial_events.is_empty() {
            Ok(String::from("Baseline captured. Recorded 0 change events."))
        } else {
            Ok(format!(
                "Daemon scan complete. Recorded {} change event{}.",
                initial_events.len(),
                super::pluralize(initial_events.len())
            ))
        };
    }

    if !had_existing_state {
        println!(
            "Baseline captured. Watching {} tracked target(s)...",
            config.tracked_paths.len()
        );
    } else if !initial_events.is_empty() {
        println!(
            "Recorded {} change event{}.",
            initial_events.len(),
            super::pluralize(initial_events.len())
        );
    }

    let (tx, rx) = mpsc::sync_channel(WORK_QUEUE_CAPACITY);
    let mut _watcher = build_watcher(WatchPlan::new(&config, paths), tx.clone())?;

    loop {
        let batch = next_work_batch(&rx)?;
        let mut state_dirty = false;
        let mut queue_reload = false;
        let mut files = BTreeSet::new();
        let mut directories = BTreeSet::new();

        for work in batch {
            match work {
                WorkItem::ReloadConfig => queue_reload = true,
                WorkItem::RefreshPath(path) => {
                    files.insert(path);
                }
                WorkItem::RefreshDirectory(path) => {
                    directories.insert(path);
                }
                WorkItem::WatcherError(err) => {
                    eprintln!("Watcher error: {err}");
                }
            }
        }

        if queue_reload {
            match app.load_or_default(scope) {
                Ok(latest) if latest != config => {
                    config = latest;
                    state.observed = merge_reloaded_observed(
                        &state.observed,
                        observe_tracked_paths(scope, &config)?,
                    );
                    app.save_daemon_state(scope, &state)?;
                    _watcher = build_watcher(WatchPlan::new(&config, paths), tx.clone())?;
                    println!(
                        "Reloaded config. Watching {} tracked target(s)...",
                        config.tracked_paths.len()
                    );
                }
                Ok(_) => {}
                Err(err) => {
                    eprintln!("Failed to reload config; keeping previous configuration: {err}");
                }
            }
        }

        let mut events = Vec::new();
        for root in &directories {
            let refreshed = refresh_directory(app, scope, &config, &mut state, root)?;
            if !refreshed.is_empty() {
                state_dirty = true;
                events.extend(refreshed);
            }
        }
        for path in files {
            if directories.iter().any(|dir| path.starts_with(dir)) {
                continue;
            }
            if let Some(event) = refresh_path(scope, &config, &mut state, &path)? {
                state_dirty = true;
                events.push(event);
            }
        }

        if !events.is_empty() {
            app.append_events(scope, &events)?;
            app.enforce_journal_retention(scope, &config.retention)?;
        }
        if state_dirty {
            app.save_daemon_state(scope, &state)?;
            println!(
                "Recorded {} change event{}.",
                events.len(),
                super::pluralize(events.len())
            );
        }
    }
}

pub fn observe_tracked_paths(
    scope: Scope,
    config: &Config,
) -> Result<BTreeMap<String, ObservedPath>> {
    let mut observed = BTreeMap::new();
    for tracked in &config.tracked_paths {
        for candidate in expand_tracked_target(scope, tracked)? {
            observed.insert(candidate.path.clone(), candidate);
        }
    }
    Ok(observed)
}

pub fn merge_reloaded_observed(
    previous: &BTreeMap<String, ObservedPath>,
    current: BTreeMap<String, ObservedPath>,
) -> BTreeMap<String, ObservedPath> {
    let mut merged = BTreeMap::new();
    for (path, current_observed) in current {
        if let Some(previous_observed) = previous.get(&path) {
            if should_preserve_observed_baseline(previous_observed, &current_observed) {
                merged.insert(path, previous_observed.clone());
            } else {
                merged.insert(path, current_observed);
            }
        } else {
            merged.insert(path, current_observed);
        }
    }
    merged
}

pub fn diff_observed(
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
        if let Some(event) = diff_pair(before, after) {
            events.push(event);
        }
    }
    events
}

pub fn retention_start_index(events: &[JournalEvent], retention: &RetentionPolicy) -> usize {
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

fn next_work_batch(rx: &Receiver<WorkItem>) -> Result<Vec<WorkItem>> {
    let first = rx
        .recv()
        .context("watcher channel disconnected; daemon worker cannot continue")?;
    let mut items = vec![first];
    while let Ok(item) = rx.try_recv() {
        items.push(item);
    }
    Ok(items)
}

fn run_full_scan(
    app: &App,
    scope: Scope,
    config: &Config,
    state: &mut DaemonState,
    had_existing_state: bool,
) -> Result<Vec<JournalEvent>> {
    let observed = observe_tracked_paths(scope, config)?;
    let events = if had_existing_state || !state.observed.is_empty() {
        diff_observed(&state.observed, &observed)
    } else {
        Vec::new()
    };
    if !events.is_empty() {
        app.append_events(scope, &events)?;
        app.enforce_journal_retention(scope, &config.retention)?;
    }
    if state.observed != observed {
        state.observed = observed;
        app.save_daemon_state(scope, state)?;
    }
    Ok(events)
}

fn refresh_directory(
    _app: &App,
    scope: Scope,
    config: &Config,
    state: &mut DaemonState,
    root: &Path,
) -> Result<Vec<JournalEvent>> {
    let Some(tracked) = config
        .tracked_paths
        .iter()
        .find(|entry| entry.kind == PathKind::Directory && Path::new(&entry.path) == root)
    else {
        return Ok(Vec::new());
    };

    let previous_keys: Vec<String> = state
        .observed
        .keys()
        .filter(|key| *key == &tracked.path || key.starts_with(&format!("{}/", tracked.path)))
        .cloned()
        .collect();
    let previous = previous_keys
        .iter()
        .filter_map(|key| {
            state
                .observed
                .get(key)
                .cloned()
                .map(|value| (key.clone(), value))
        })
        .collect::<BTreeMap<_, _>>();
    let current = expand_tracked_target(scope, tracked)?
        .into_iter()
        .map(|observed| (observed.path.clone(), observed))
        .collect::<BTreeMap<_, _>>();

    let events = diff_observed(&previous, &current);
    for key in previous_keys {
        state.observed.remove(&key);
    }
    for (key, observed) in current {
        state.observed.insert(key, observed);
    }
    Ok(events)
}

fn refresh_path(
    scope: Scope,
    config: &Config,
    state: &mut DaemonState,
    path: &Path,
) -> Result<Option<JournalEvent>> {
    let Some(tracked) = tracked_entry_for_observed_path(config, path) else {
        return Ok(None);
    };

    let key = normalize_display_path(path);
    let observed = observe_single_path(
        scope,
        path.to_path_buf(),
        tracked.category,
        tracked.diff_mode,
        tracked.redaction,
    )?;
    let previous = state.observed.get(&key).cloned();
    let event = diff_pair(previous.as_ref(), Some(&observed));

    if previous.is_some() || observed.exists {
        state.observed.insert(key, observed);
    }

    Ok(event)
}

fn tracked_entry_for_observed_path<'a>(config: &'a Config, path: &Path) -> Option<&'a TrackedPath> {
    let rendered = normalize_display_path(path);
    let mut matched = None;
    for entry in &config.tracked_paths {
        match entry.kind {
            PathKind::File if entry.path == rendered => matched = Some(entry),
            PathKind::Directory
                if rendered == entry.path || rendered.starts_with(&format!("{}/", entry.path)) =>
            {
                matched = Some(entry);
            }
            _ => {}
        }
    }
    matched
}

fn diff_pair(before: Option<&ObservedPath>, after: Option<&ObservedPath>) -> Option<JournalEvent> {
    match (before, after) {
        (None, Some(after)) if after.exists => {
            Some(build_event(EventKind::Created, None, Some(after)))
        }
        (Some(before), None) if before.exists => {
            Some(build_event(EventKind::Removed, Some(before), None))
        }
        (Some(before), Some(after)) => {
            if before.exists != after.exists {
                if after.exists {
                    Some(build_event(EventKind::Created, Some(before), Some(after)))
                } else {
                    Some(build_event(EventKind::Removed, Some(before), Some(after)))
                }
            } else if before.fingerprint != after.fingerprint {
                Some(build_event(EventKind::Modified, Some(before), Some(after)))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn expand_tracked_target(scope: Scope, tracked: &TrackedPath) -> Result<Vec<ObservedPath>> {
    let path = PathBuf::from(&tracked.path);
    match tracked.kind {
        PathKind::File => Ok(vec![observe_single_path(
            scope,
            path,
            tracked.category,
            tracked.diff_mode,
            tracked.redaction,
        )?]),
        PathKind::Directory => {
            if !path.exists() {
                return Ok(vec![observe_single_path(
                    scope,
                    path,
                    tracked.category,
                    tracked.diff_mode,
                    tracked.redaction,
                )?]);
            }

            let mut entries = Vec::new();
            for entry in WalkDir::new(&path)
                .into_iter()
                .filter_map(|entry| entry.ok())
            {
                if entry.file_type().is_dir() {
                    continue;
                }
                entries.push(observe_single_path(
                    scope,
                    entry.path().to_path_buf(),
                    tracked.category,
                    tracked.diff_mode,
                    tracked.redaction,
                )?);
            }
            if entries.is_empty() {
                entries.push(observe_single_path(
                    scope,
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
    scope: Scope,
    path: PathBuf,
    category: Category,
    diff_mode: DiffMode,
    redaction: RedactionMode,
) -> Result<ObservedPath> {
    let display = normalize_display_path(&path);
    if !path.exists() {
        return Ok(ObservedPath {
            scope,
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
            scope,
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
        scope,
        path: display,
        category,
        diff_mode,
        redaction,
        exists: true,
        fingerprint: Some(fingerprint),
        text_snapshot,
    })
}

fn build_event(
    kind: EventKind,
    before: Option<&ObservedPath>,
    after: Option<&ObservedPath>,
) -> JournalEvent {
    let reference = after
        .or(before)
        .expect("an event requires a reference path");
    let diff = match kind {
        EventKind::Modified | EventKind::Created => build_diff(before, after),
        EventKind::Removed => None,
    };
    let (added_lines, removed_lines) = diff_line_counts(diff.as_deref());
    JournalEvent {
        timestamp: OffsetDateTime::now_utc(),
        scope: reference.scope,
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
    let old = before
        .and_then(|entry| entry.text_snapshot.as_deref())
        .unwrap_or("");
    let new = after
        .and_then(|entry| entry.text_snapshot.as_deref())
        .unwrap_or("");
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

fn summarize_event(
    kind: &EventKind,
    category: Category,
    added_lines: usize,
    removed_lines: usize,
) -> String {
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

fn should_preserve_observed_baseline(previous: &ObservedPath, current: &ObservedPath) -> bool {
    previous.scope == current.scope
        && previous.path == current.path
        && previous.category == current.category
        && previous.diff_mode == current.diff_mode
        && previous.redaction == current.redaction
}

pub(crate) fn maybe_redact_text(text: String, redaction: RedactionMode) -> String {
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
            url_auth
                .replace_all(line, "${scheme}[REDACTED]@")
                .into_owned()
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

fn system_time_secs(value: SystemTime) -> Option<u64> {
    value
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}
