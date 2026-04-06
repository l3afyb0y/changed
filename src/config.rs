use crate::category::Category;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Default)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub retention: RetentionPolicy,
    #[serde(default)]
    pub tracked_paths: Vec<TrackedPath>,
    #[serde(default)]
    pub tracked_packages: Vec<TrackedPackage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct TrackedPath {
    pub path: String,
    pub category: Category,
    pub kind: PathKind,
    pub diff_mode: DiffMode,
    pub redaction: RedactionMode,
    pub source: TrackSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct TrackedPackage {
    pub manager: String,
    pub package_name: String,
    pub source: TrackSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RetentionPolicy {
    pub max_events: usize,
    pub max_bytes: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiffMode {
    MetadataOnly,
    Unified,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionMode {
    Off,
    Auto,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrackSource {
    Manual,
    Preset,
}

impl Config {
    pub fn new() -> Self {
        Self {
            version: 1,
            retention: RetentionPolicy::default(),
            tracked_paths: Vec::new(),
            tracked_packages: Vec::new(),
        }
    }

    pub fn sort_and_dedup(&mut self) {
        // Sort by path first so adjacent dedup_by works correctly across all categories,
        // then re-sort by the intended display order (category, path, kind).
        self.tracked_paths.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| a.category.cmp(&b.category))
                .then_with(|| path_kind_rank(a.kind).cmp(&path_kind_rank(b.kind)))
        });
        self.tracked_paths.dedup_by(|a, b| a.path == b.path);
        self.tracked_paths.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| path_kind_rank(a.kind).cmp(&path_kind_rank(b.kind)))
        });

        self.tracked_packages.sort_by(|a, b| {
            a.manager
                .cmp(&b.manager)
                .then_with(|| a.package_name.cmp(&b.package_name))
        });
        self.tracked_packages
            .dedup_by(|a, b| a.manager == b.manager && a.package_name == b.package_name);
    }
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_events: 10_000,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

fn path_kind_rank(kind: PathKind) -> u8 {
    match kind {
        PathKind::File => 0,
        PathKind::Directory => 1,
    }
}
