# Categories

This document defines the initial category set for `changed`.

These categories are meant to stay small, memorable, and useful for tuning
history. They describe why a change matters, not just where a file lives.

For v1, each tracked path should have one primary category.

## Rules

- Categories should reflect tuning domains, not generic filesystem buckets.
- A category should be easy to understand from the CLI alone.
- Categories can grow over time, but new categories should only be added when
  real usage shows a gap.
- Presets may auto-enable tracked paths, but they should map back into this
  shared category list.
- `packages` is reserved for optional package-event tracking and should remain
  off by default.

## V1 Categories

### `cpu`

CPU-related system tuning.

Use for:

- CPU governor and power policy config
- CPU-specific module options
- microcode-adjacent OS tuning
- CPU-targeted kernel or userspace performance tuning

Example paths:

- `/etc/default/cpupower`
- `/etc/modprobe.d/amd-pstate.conf`
- `/etc/modprobe.d/intel-pstate.conf`

### `gpu`

Graphics and display-stack tuning.

Use for:

- AMD, NVIDIA, or Intel GPU config
- graphics driver module options
- compositor or display tuning tied to GPU behavior

Example paths:

- `/etc/modprobe.d/nvidia.conf`
- `/etc/X11/xorg.conf.d/20-amdgpu.conf`
- `/etc/environment`

### `services`

Service and daemon configuration.

Use for:

- `systemd` unit files
- service overrides
- service-related config files
- daemon-specific tuning that is mainly about how a service runs

Example paths:

- `/etc/systemd/system/example.service`
- `/etc/systemd/system.conf`
- `~/.config/systemd/user/example.service`

### `scheduler`

Scheduling and latency/performance tuning.

Use for:

- I/O scheduler config
- scheduler-focused `sysctl` tuning
- process scheduling and responsiveness tuning

Example paths:

- `/etc/sysctl.d/99-scheduler.conf`
- `/etc/udev/rules.d/60-ioschedulers.rules`

### `shell`

Shell behavior and interactive environment tuning.

Use for:

- `fish`, `bash`, and `zsh` config
- aliases and shell functions
- prompt and shell startup behavior
- environment-adjacent shell config

Example paths:

- `~/.config/fish/config.fish`
- `~/.bashrc`
- `~/.zshrc`

Note:

Shell files may contain secrets or exported values. They should default to
safer tracking modes unless the user explicitly enables line diffs.

### `build`

Build and toolchain tuning.

Use for:

- `makepkg` config
- compiler flag tuning
- build cache and packaging tool config

Example paths:

- `/etc/makepkg.conf`
- `~/.config/pacman/makepkg.conf`

### `boot`

Boot and early-startup tuning.

Use for:

- bootloader config
- initramfs config
- kernel command line config

Example paths:

- `/etc/default/grub`
- `/etc/mkinitcpio.conf`
- `/boot/loader/entries/*.conf`

### `audio`

Audio-stack tuning.

Use for:

- PipeWire configuration
- WirePlumber policy config
- ALSA or PulseAudio tuning

Example paths:

- `~/.config/pipewire/pipewire.conf`
- `~/.config/wireplumber/wireplumber.conf.d/*.conf`
- `/etc/pipewire/pipewire.conf`

### `packages`

Optional package history tracking.

Use for:

- package installs
- package removals
- package replacements

Do not use by default for:

- routine package updates

This category is reserved for later event-source support and should be disabled
by default in early releases.

## Non-Goals

The following should not be added as broad categories in v1:

- `system`
- `hardware`
- `desktop`

These are too vague and would quickly turn into catch-all buckets.

## Future Growth

If a manually tracked path keeps proving useful, it can later be promoted into
a default preset within one of these categories.

That should be the normal way the category presets evolve over time: by real
usage, not by trying to predict every future need up front.
