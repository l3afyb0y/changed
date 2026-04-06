# Default Tracked Paths

This document lists the preset-backed paths that `changed` knows about today.

Two rules matter:

- `changed init` seeds the base preset set for one scope
- `sudo changed setup` adds the setup-only preset set for both scopes

Every preset is existence-gated. If a path does not exist on the current
machine, it is skipped silently.

## Base Presets

These are the presets loaded by `changed init`, and they remain part of normal
default tracking without requiring `changed setup`.

### System Scope

- `/etc/systemd/system.conf`
- `/etc/environment`
- `/etc/dhcpcd.conf`
- `/etc/sudo.conf`
- `/etc/bash.bashrc`
- `/etc/fstab`
- `/etc/default/grub`
- `/etc/mkinitcpio.conf`
- `/boot/loader/entries/`

### User Scope

- `~/.config/systemd/user/`
- `~/.config/fish/config.fish`
- `~/.bashrc`
- `~/.zshrc`
- `~/.config/pipewire/pipewire.conf`
- `~/.config/wireplumber/wireplumber.conf.d/`

## Setup-Added Presets

These are added by `sudo changed setup` when they exist.

### System Scope

- `/etc/default/cpupower`
- `/etc/modprobe.d/amd-pstate.conf`
- `/etc/intel-undervolt.conf`
- `/etc/default/cpupower-service.conf`
- `/etc/systemd/system/intel-pstate-pin.service`
- `/etc/gamemode.ini`
- `/etc/modprobe.d/nvidia.conf`
- `/etc/udev/rules.d/99-nvidia-irq-affinity.rules`
- `/etc/nvidia_oc.json`
- `/etc/X11/xorg.conf.d/20-amdgpu.conf`
- `/etc/X11/xorg.conf.d/20-intel.conf`
- `/etc/sysctl.d/99-scheduler.conf`
- `/etc/udev/rules.d/60-ioschedulers.rules`
- `/etc/makepkg.conf`
- `/etc/ccache.conf`
- `/etc/pacman.conf`
- `/etc/makepkg.conf.d/`
- `/etc/makepkg.d/`
- `/etc/pipewire/pipewire.conf`

### User Scope

- `~/.config/hypr/`
- `~/.config/pacman/makepkg.conf`

## Notes

- Directory presets track files under that directory, not just the directory
  entry itself.
- Shell presets default to metadata-only diffing with automatic redaction.
- Many system tuning presets default to unified diffs with redaction disabled
  because they are expected to be explicit configuration files.
- If you want something outside these presets, add it manually with
  `changed track [args]`.
