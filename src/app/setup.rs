use crate::setup::{CpuVendor, GpuVendor, SetupProfile, ShellKind};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub fn detect_setup_profile() -> SetupProfile {
    let mut shells = BTreeSet::new();
    if let Some(shell) = std::env::var_os("SHELL") {
        let shell = shell.to_string_lossy();
        if shell.contains("fish") {
            shells.insert(ShellKind::Fish);
        } else if shell.contains("zsh") {
            shells.insert(ShellKind::Zsh);
        } else if shell.contains("bash") {
            shells.insert(ShellKind::Bash);
        }
    }

    let mut gpu_vendors = BTreeSet::new();
    if let Some(vendor) = detect_gpu_vendor() {
        gpu_vendors.insert(vendor);
    }

    SetupProfile {
        version: 1,
        cpu_vendor: detect_cpu_vendor(),
        gpu_vendors: gpu_vendors.into_iter().collect(),
        shells: shells.into_iter().collect(),
    }
}

fn detect_cpu_vendor() -> Option<CpuVendor> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
    if cpuinfo.contains("AuthenticAMD") {
        Some(CpuVendor::Amd)
    } else if cpuinfo.contains("GenuineIntel") {
        Some(CpuVendor::Intel)
    } else {
        None
    }
}

fn detect_gpu_vendor() -> Option<GpuVendor> {
    if Path::new("/sys/module/nvidia").exists() {
        return Some(GpuVendor::Nvidia);
    }

    let vendor_file = "/sys/class/drm/card0/device/vendor";
    let vendor = fs::read_to_string(vendor_file).ok()?;
    match vendor.trim() {
        "0x1002" => Some(GpuVendor::Amd),
        "0x10de" => Some(GpuVendor::Nvidia),
        "0x8086" => Some(GpuVendor::Intel),
        _ => None,
    }
}
