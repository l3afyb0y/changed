use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize, ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Cpu,
    Gpu,
    Services,
    Scheduler,
    Shell,
    Build,
    Boot,
    Audio,
    Packages,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::Cpu,
        Category::Gpu,
        Category::Services,
        Category::Scheduler,
        Category::Shell,
        Category::Build,
        Category::Boot,
        Category::Audio,
        Category::Packages,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Cpu => "cpu",
            Category::Gpu => "gpu",
            Category::Services => "services",
            Category::Scheduler => "scheduler",
            Category::Shell => "shell",
            Category::Build => "build",
            Category::Boot => "boot",
            Category::Audio => "audio",
            Category::Packages => "packages",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Category::Cpu => "CPU-related system tuning",
            Category::Gpu => "Graphics and display-stack tuning",
            Category::Services => "Service and daemon configuration",
            Category::Scheduler => "Scheduling and latency/performance tuning",
            Category::Shell => "Shell behavior and interactive environment tuning",
            Category::Build => "Build and toolchain tuning",
            Category::Boot => "Boot and early-startup tuning",
            Category::Audio => "Audio-stack tuning",
            Category::Packages => "Optional package history tracking",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
