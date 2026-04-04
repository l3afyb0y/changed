# changed

`changed` is a lightweight system tuning changelog with a dedicated daemon for
systemd-based systems.

It is built to answer:

- What did I change over time?
- Which changes mattered for CPU, GPU, services, shell, boot, or build tuning?
- What do I need to carry forward to a new install or new hardware?

It is not a backup, rollback, or recovery tool. Its purpose is to make system
configuration changes easier to audit and reproduce.

## Current Status

The core tracking loop exists today and is usable for local development:

- `changed` manages user and system scoped config, tracked targets, history
  views, and diff/redaction policy
- `changedd` captures a baseline, watches one selected scope, and appends
  journal events
- `changed service` can now install, start, stop, and inspect scoped systemd
  services

## Current Shape

The project currently has two binaries:

- `changed`: user-facing CLI for tracking targets, browsing history, and
  managing diff/redaction policy
- `changedd`: dedicated daemon for watching tracked paths and appending
  journal events in either user or system scope

The daemon is event-driven through `notify`, with scan/diff verification layered
underneath for correctness. It captures an initial baseline, then records
subsequent file-backed changes while it runs.

## Build And Run

With two binaries in the workspace, `cargo run` needs an explicit target:

```bash
cargo build
cargo run --bin changed -- --help
cargo run --bin changedd -- --help
```

Typical local development flow:

```bash
cargo run --bin changed -- init -U
cargo run --bin changed -- init -S
cargo run --bin changed -- track -U ~/.config/fish/config.fish
sudo cargo run --bin changed -- track -S /boot/loader/entries/arch.conf
cargo run --bin changedd -- --user --once
sudo cargo run --bin changedd -- --system --once
cargo run --bin changed -- list -U -C
sudo cargo run --bin changed -- list -SU -a
```

## Project Direction

The project now supports two scopes of tracking:

- system scope for machine-wide tuning such as `/etc`, `/boot`, loader entries,
  systemd units, and kernel cmdline files
- user scope for per-user files such as `~/.config`, shell config, and
  user-level services

This is meant to support:

- a system log visible to privileged users
- one private user log per user
- separate system and user services, both using the same `changedd` binary

See [docs/scope-model.md](docs/scope-model.md) for the detailed design.

## What Works Today

- Track exact files in user or system scope
- Track preset categories like `shell`, `build`, or `services`
- Infer scope for path-based writes when the path is obviously user or system
- Enable or disable diff storage per path
- Enable or disable redaction per path
- Filter history with `-i/--include` and `-e/--exclude`
- Read merged history with `-SU` or with no scope flags
- Auto-reload daemon config when tracked targets change
- Append a structured journal and render it as a readable changelog
- Show a low-noise daily view with `changed list -C`
- Install and control scoped systemd services from the CLI

## What Does Not Exist Yet

- package event tracking
- service state event tracking
- a dedicated `status` command for daemon diagnostics

## Scope-Aware CLI

Read commands support composable scope flags:

- `-S, --system`
- `-U, --user`

Examples:

```bash
changed list
changed list -U
sudo changed list -S
sudo changed list -SU -a -C
```

If system scope is requested without privilege, `changed` now fails clearly and
asks you to re-run with `sudo` or narrow to `-U`.

List filtering uses:

- `-i, --include <CATEGORY>`
- `-e, --exclude <CATEGORY>`

Examples:

```bash
changed list -i services
changed list -e packages
changed list -SU -C -i cpu -i gpu -e services
```

For write operations such as `track`, `untrack`, `diff`, and `redact`:

- exactly one scope is targeted
- path-based writes may infer scope when the path is obvious
- if scope is unclear, the command fails and asks for `-S` or `-U`
- category and package writes should be given an explicit scope

## Service Integration

Service commands now require an explicit scope:

- `changed service -U install`
- `changed service -U start`
- `changed service -U stop`
- `changed service -U status`
- `sudo changed service -S install`
- `sudo changed service -S start`

Behavior:

- `install` writes a scope-appropriate unit file and runs `daemon-reload`
- `start` runs `systemctl enable --now` for that scope
- `stop` runs `systemctl disable --now` for that scope
- `status` shows `systemctl status` output for that scope

For local development installs, unit files are generated dynamically and point
at the sibling `changedd` binary next to the current `changed` executable.

## Journal Behavior

- The first daemon scan captures a baseline without emitting synthetic events.
- New targets added while the daemon is running are auto-reloaded and silently
  baselined.
- The journal keeps historical changes instead of replacing old entries with
  newer ones.
- `changed list` shows a recent view by default. Use `changed list -a` for the
  full retained journal.
- `changed list` with no scope flags reads merged system plus current-user
  history.
- `-C, --clean-view` changes presentation only. It does not delete or rewrite
  history.

## Retention

The journal currently keeps a bounded local history using config defaults:

- `max_events = 10000`
- `max_bytes = 8 MiB`

Retention trims the oldest stored events when limits are exceeded. This keeps
the local journal lightweight, but it also means very old history will
eventually roll off unless archival support is added later.

## Config Shape

The current config is stored as TOML at `~/.config/changed/config.toml`.

Example:

```toml
version = 1

[retention]
max_events = 10000
max_bytes = 8388608

[[tracked_paths]]
path = "/etc/makepkg.conf"
category = "build"
kind = "file"
diff_mode = "unified"
redaction = "off"
source = "manual"

[[tracked_paths]]
path = "/home/rowen/.config/fish/config.fish"
category = "shell"
kind = "file"
diff_mode = "metadata_only"
redaction = "auto"
source = "preset"
```

System scope uses separate config and state roots:

- config: `/etc/changed/config.toml`
- state: `/var/lib/changed/`

## Safety Model

Tracking, diffing, and redaction are separate controls:

- Tracking decides whether a target is watched.
- Diff decides whether readable line changes are stored.
- Redaction decides whether stored diffs should mask likely sensitive values.

Shell and environment-adjacent files default to safer tracking behavior.
Redaction is heuristic, not perfect, so highly sensitive files should still be
handled carefully.

The current security direction is:

- system journals and state should be root-owned and root-readable only
- user journals, config, and state should be user-owned and private
- user-scope files should not require `sudo` just to be tracked
- shell-like files should remain conservative by default

This keeps logs private to their scope owner without forcing the entire CLI
behind `sudo`.

## Files

User scope:

- Config: `~/.config/changed/config.toml`
- State: `~/.local/state/changed/`
- Journal: `~/.local/state/changed/journal.jsonl`
- Daemon state: `~/.local/state/changed/daemon-state.json`

System scope:

- Config: `/etc/changed/config.toml`
- State: `/var/lib/changed/`
- Journal: `/var/lib/changed/journal.jsonl`
- Daemon state: `/var/lib/changed/daemon-state.json`

Environment overrides:

- `CHANGED_CONFIG_HOME`
- `CHANGED_STATE_HOME`
- `CHANGED_SYSTEM_CONFIG_HOME`
- `CHANGED_SYSTEM_STATE_HOME`

## Example Output

For the intended human-readable changelog style, see [example-log.md](example-log.md).

## Documentation

- [Scope and security model](docs/scope-model.md)
- [CLI help drafts](docs/help-text.md)
- [Man-page-style reference](docs/changed.1.md)
- [Category definitions](docs/categories.md)
- [Packaging workflow](docs/packaging-workflow.md)

## Arch Packaging

This repo now includes a local-source [PKGBUILD](PKGBUILD) and packaged unit
files under
[packaging/systemd/system/changedd.service](packaging/systemd/system/changedd.service)
and
[packaging/systemd/user/changedd.service](packaging/systemd/user/changedd.service).

The PKGBUILD installs:

- `changed`
- `changedd`
- both systemd unit files
- project documentation and license files

After installing the package, the units are already present under
`/usr/lib/systemd`. You can enable them directly:

```bash
sudo systemctl enable --now changedd.service
systemctl --user enable --now changedd.service
```

`changed service install` is mainly for local development or non-packaged
installs where the unit should be generated dynamically from the current binary
location.
