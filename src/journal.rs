use crate::category::Category;
use crate::scope::Scope;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Created,
    Modified,
    Removed,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct JournalEvent {
    pub timestamp: OffsetDateTime,
    #[serde(default)]
    pub scope: Scope,
    pub kind: EventKind,
    pub category: Category,
    pub path: String,
    pub summary: String,
    #[serde(default)]
    pub added_lines: usize,
    #[serde(default)]
    pub removed_lines: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}
