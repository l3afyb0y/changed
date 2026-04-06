use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Default)]
pub struct SetupProfile {
    pub version: u32,
    #[serde(default)]
    pub cpu_vendor: Option<CpuVendor>,
    #[serde(default)]
    pub gpu_vendors: Vec<GpuVendor>,
    #[serde(default)]
    pub shells: Vec<ShellKind>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CpuVendor {
    Intel,
    Amd,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ShellKind {
    Bash,
    Fish,
    Zsh,
}

impl SetupProfile {
    pub fn sort_and_dedup(&mut self) {
        self.gpu_vendors.sort();
        self.gpu_vendors.dedup();
        self.shells.sort();
        self.shells.dedup();
        if self.version == 0 {
            self.version = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_profile_round_trips_with_intel_and_nvidia() {
        let profile = SetupProfile {
            version: 1,
            cpu_vendor: Some(CpuVendor::Intel),
            gpu_vendors: vec![GpuVendor::Nvidia],
            shells: vec![ShellKind::Fish],
        };

        let encoded = toml::to_string(&profile).expect("profile should serialize");
        let decoded: SetupProfile = toml::from_str(&encoded).expect("profile should deserialize");

        assert_eq!(decoded, profile);
    }
}
