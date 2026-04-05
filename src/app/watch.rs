use crate::config::{Config, PathKind};
use anyhow::{Context, Result};
use notify::{
    RecommendedWatcher, RecursiveMode, Watcher,
    event::{
        AccessKind, CreateKind, DataChange, EventKind as NotifyEventKind, MetadataKind, ModifyKind,
        RemoveKind, RenameMode,
    },
    recommended_watcher,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;

use super::paths::AppPaths;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum WorkItem {
    ReloadConfig,
    RefreshPath(PathBuf),
    RefreshDirectory(PathBuf),
    WatcherError(String),
}

#[derive(Clone, Debug)]
pub struct WatchPlan {
    config_file: PathBuf,
    roots: Vec<WatchRoot>,
    tracked_files: BTreeSet<PathBuf>,
    tracked_dirs: BTreeSet<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct WatchRootSummary {
    pub path: PathBuf,
    pub recursive: bool,
}

pub struct ActiveWatcher {
    _watcher: RecommendedWatcher,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct WatchRoot {
    path: PathBuf,
    recursive: bool,
}

impl WatchPlan {
    pub fn new(config: &Config, paths: &AppPaths) -> Self {
        let mut roots = BTreeMap::new();
        insert_watch_root(
            &mut roots,
            paths
                .config_file()
                .parent()
                .map_or_else(|| paths.config_home.clone(), Path::to_path_buf),
            false,
        );

        let mut tracked_files = BTreeSet::from([paths.config_file()]);
        let mut tracked_dirs = BTreeSet::new();
        for tracked in &config.tracked_paths {
            let path = PathBuf::from(&tracked.path);
            match tracked.kind {
                PathKind::Directory => {
                    insert_watch_root(&mut roots, path.clone(), true);
                    tracked_dirs.insert(path);
                }
                PathKind::File => {
                    let watch_path = path
                        .parent()
                        .map_or_else(|| path.clone(), Path::to_path_buf);
                    insert_watch_root(&mut roots, watch_path, false);
                    tracked_files.insert(path);
                }
            }
        }

        Self {
            config_file: paths.config_file(),
            roots: roots
                .into_iter()
                .map(|(path, recursive)| WatchRoot { path, recursive })
                .collect(),
            tracked_files,
            tracked_dirs,
        }
    }

    pub fn root_summaries(&self) -> Vec<WatchRootSummary> {
        self.roots
            .iter()
            .map(|root| WatchRootSummary {
                path: root.path.clone(),
                recursive: root.recursive,
            })
            .collect()
    }
}

pub fn build_watcher(plan: WatchPlan, tx: SyncSender<WorkItem>) -> Result<ActiveWatcher> {
    let event_plan = plan.clone();
    let mut watcher =
        recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event) => {
                for item in map_event(&event_plan, &event.kind, &event.paths) {
                    let _ = tx.send(item);
                }
            }
            Err(err) => {
                let _ = tx.send(WorkItem::WatcherError(err.to_string()));
            }
        })
        .context("failed to create filesystem watcher")?;

    for root in &plan.roots {
        let recursive_mode = if root.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        watcher
            .watch(&root.path, recursive_mode)
            .with_context(|| format!("failed to watch {}", root.path.display()))?;
    }

    Ok(ActiveWatcher { _watcher: watcher })
}

fn map_event(plan: &WatchPlan, kind: &NotifyEventKind, paths: &[PathBuf]) -> Vec<WorkItem> {
    if !is_relevant_notify_event(kind) {
        return Vec::new();
    }

    let mut items = BTreeSet::new();
    for path in paths {
        if path == &plan.config_file {
            items.insert(WorkItem::ReloadConfig);
            continue;
        }

        if plan.tracked_files.contains(path) {
            items.insert(WorkItem::RefreshPath(path.clone()));
            continue;
        }

        for root in &plan.tracked_dirs {
            if path == root {
                items.insert(WorkItem::RefreshDirectory(root.clone()));
            } else if path.starts_with(root) {
                items.insert(WorkItem::RefreshPath(path.clone()));
            }
        }
    }

    items.into_iter().collect()
}

fn is_relevant_notify_event(kind: &NotifyEventKind) -> bool {
    matches!(
        kind,
        NotifyEventKind::Any
            | NotifyEventKind::Create(
                CreateKind::Any | CreateKind::File | CreateKind::Folder | CreateKind::Other
            )
            | NotifyEventKind::Modify(ModifyKind::Any)
            | NotifyEventKind::Modify(ModifyKind::Data(
                DataChange::Any | DataChange::Content | DataChange::Size | DataChange::Other
            ))
            | NotifyEventKind::Modify(ModifyKind::Metadata(
                MetadataKind::Any
                    | MetadataKind::WriteTime
                    | MetadataKind::Permissions
                    | MetadataKind::Ownership
                    | MetadataKind::Extended
                    | MetadataKind::Other
            ))
            | NotifyEventKind::Modify(ModifyKind::Name(
                RenameMode::Any
                    | RenameMode::To
                    | RenameMode::From
                    | RenameMode::Both
                    | RenameMode::Other
            ))
            | NotifyEventKind::Modify(ModifyKind::Other)
            | NotifyEventKind::Remove(
                RemoveKind::Any | RemoveKind::File | RemoveKind::Folder | RemoveKind::Other
            )
    ) && !matches!(
        kind,
        NotifyEventKind::Access(
            AccessKind::Any
                | AccessKind::Read
                | AccessKind::Open(_)
                | AccessKind::Close(_)
                | AccessKind::Other
        )
    )
}

fn insert_watch_root(roots: &mut BTreeMap<PathBuf, bool>, path: PathBuf, recursive: bool) {
    roots
        .entry(path)
        .and_modify(|existing| *existing |= recursive)
        .or_insert(recursive);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::category::Category;
    use crate::config::{DiffMode, RedactionMode, RetentionPolicy, TrackSource, TrackedPath};
    use crate::scope::Scope;

    #[test]
    fn watch_plan_uses_parent_dirs_for_files_and_recursive_for_directories() {
        let paths = AppPaths {
            scope: Scope::User,
            config_home: PathBuf::from("/tmp/config-home"),
            state_home: PathBuf::from("/tmp/state-home"),
        };
        let config = Config {
            version: 1,
            retention: RetentionPolicy::default(),
            tracked_paths: vec![
                TrackedPath {
                    path: "/home/test/.config/fish/config.fish".to_owned(),
                    category: Category::Shell,
                    kind: PathKind::File,
                    diff_mode: DiffMode::Unified,
                    redaction: RedactionMode::Auto,
                    source: TrackSource::Manual,
                },
                TrackedPath {
                    path: "/home/test/.config/fish/conf.d".to_owned(),
                    category: Category::Shell,
                    kind: PathKind::Directory,
                    diff_mode: DiffMode::Unified,
                    redaction: RedactionMode::Auto,
                    source: TrackSource::Manual,
                },
            ],
            tracked_packages: Vec::new(),
        };

        let plan = WatchPlan::new(&config, &paths);

        assert!(plan.roots.contains(&WatchRoot {
            path: PathBuf::from("/tmp/config-home"),
            recursive: false,
        }));
        assert!(plan.roots.contains(&WatchRoot {
            path: PathBuf::from("/home/test/.config/fish"),
            recursive: false,
        }));
        assert!(plan.roots.contains(&WatchRoot {
            path: PathBuf::from("/home/test/.config/fish/conf.d"),
            recursive: true,
        }));
    }

    #[test]
    fn event_mapping_accepts_only_tracked_files_or_directory_descendants() {
        let plan = WatchPlan {
            config_file: PathBuf::from("/tmp/config-home/config.toml"),
            roots: Vec::new(),
            tracked_files: BTreeSet::from([
                PathBuf::from("/tmp/config-home/config.toml"),
                PathBuf::from("/home/test/.config/fish/config.fish"),
            ]),
            tracked_dirs: BTreeSet::from([PathBuf::from("/home/test/.config/fish/conf.d")]),
        };

        assert_eq!(
            map_event(
                &plan,
                &NotifyEventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &[PathBuf::from("/home/test/.config/fish/config.fish")]
            ),
            vec![WorkItem::RefreshPath(PathBuf::from(
                "/home/test/.config/fish/config.fish"
            ))]
        );
        assert_eq!(
            map_event(
                &plan,
                &NotifyEventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &[PathBuf::from("/home/test/.config/fish/conf.d/plugin.fish")]
            ),
            vec![WorkItem::RefreshPath(PathBuf::from(
                "/home/test/.config/fish/conf.d/plugin.fish"
            ))]
        );
        assert!(
            map_event(
                &plan,
                &NotifyEventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &[PathBuf::from("/home/test/.config/fish/fish_variables")]
            )
            .is_empty()
        );
    }
}
