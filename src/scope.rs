use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    System,
    #[default]
    User,
}

impl Scope {
    pub const ALL: [Scope; 2] = [Scope::System, Scope::User];

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::System => "system",
            Scope::User => "user",
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
