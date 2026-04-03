# CHANGED(1)

## NAME

`changed` - lightweight system tuning changelog for Arch Linux

## SYNOPSIS

`changed <command> [options]`

Dedicated daemon binary:

`changedd [options]`

## DESCRIPTION

`changed` records a readable history of system tuning and configuration
changes while its daemon is running.

It is designed to answer:

- What did I change over time?
- Which changes were related to CPU, GPU, services, shell, boot, or other
  tuning?
- What do I need to carry forward to a new install or new hardware?

`changed` is not a backup, rollback, snapshot, or recovery tool.

## STATUS

The current codebase already provides:

- a working CLI
- a dedicated daemon binary
- file-backed journaling
- diff and redaction controls
- category-aware rendering

The service layer and final scope-aware CLI are still being finalized before
systemd integration.

## SCOPE MODEL

The intended design now has two scopes:

- system scope
- user scope

System scope is for machine-wide tuning such as:

- `/etc`
- `/boot/loader/entries`
- system units
- system-wide scheduler or boot config

User scope is for per-user tuning such as:

- `~/.config`
- shell startup files
- user services
- user audio or desktop tuning

The intended scope flags are:

- `-S`, `--system`
- `-U`, `--user`

For read commands, no scope flag should default to merged output from both
system scope and the current user's scope.

For write commands, exactly one scope should be targeted. If scope is not
obvious and cannot be inferred safely, the command should fail and ask the user
to specify `-S` or `-U`.

See also [docs/scope-model.md](scope-model.md).

## COMMANDS

### `init`

Initialize configuration, state directories, and default tracking presets.

### `daemon`

Run the tracking daemon in the foreground.

Supported today:

- `--once`
- `--interval-seconds <seconds>`

For long-running daemon use, prefer the dedicated binary `changedd`.

### `service <action>`

Manage the systemd service.

Planned actions:

- `install`
- `start`
- `stop`
- `status`

This interface is intentionally present before systemd integration lands.

### `track`

Track a file path, category preset, or package target.

Planned direction:

- `changed track -S /boot/loader/entries/arch.conf`
- `changed track -U ~/.config/fish/config.fish`
- `changed track -U category shell`

### `untrack`

Remove a tracked file path, category preset, or package target.

Writes should target one scope only.

### `list`

Show the changelog or tracked targets.

Current implementation includes:

- `--tracked`, `-t`
- `--path`, `-p <file_path>`
- `--all`, `-a`
- `--since`, `-s <time>`
- `--until`, `-u <time>`
- `--clean-view`, `-C`

The next CLI revision is expected to replace category filtering with:

- `--include`, `-i <category>`
- `--exclude`, `-e <category>`

And add scope filtering with:

- `--system`, `-S`
- `--user`, `-U`

### `diff <action> <path>`

Control line-diff storage for a tracked path.

Supported actions:

- `enable`
- `disable`

### `redact <action> <path>`

Control automatic redaction for a tracked path.

Supported actions:

- `enable`
- `disable`

## LIST BEHAVIOR

`changed list` is the primary read command.

Default output should:

- show a recent slice of history
- group entries in a readable changelog style
- merge system plus current-user scopes once the scope model lands

`changed list -a` should show the full retained history.

`changed list -C` should provide a low-noise day-to-day reading mode.

`--clean-view` changes presentation only. It must not delete data, alter
history, or change what the daemon stores.

The intended category filter semantics are:

- `-i`, `--include` means only these categories
- `-e`, `--exclude` removes categories from the result
- repeated filters are allowed
- exclusion wins if both are present

## DAEMON BEHAVIOR

The daemon currently uses an event-driven watcher with scan/diff verification.

- The first daemon run captures a baseline without emitting synthetic events.
- Config changes are reloaded automatically.
- Newly added tracked targets are silently baselined on reload.
- Existing tracked targets preserve prior observation state across reloads.

The long-term service design is expected to use the same `changedd` binary for:

- one system service
- one optional user service per user

## SECURITY MODEL

Tracking, diffing, and redaction are separate controls.

- Tracking decides whether a target is watched.
- Diff decides whether readable line changes are stored.
- Redaction decides whether stored diffs should mask likely sensitive values.

The current security direction is:

- system journals and state should be root-owned and root-readable only
- user journals, state, and config should be private to the owning user
- user-scope workflows should not require `sudo` by default
- shell and environment-adjacent files should stay conservative by default

Redaction is heuristic, not perfect. Highly sensitive files should still be
handled carefully.

## DEVELOPMENT

With two binaries in the workspace, local cargo runs should be explicit:

- `cargo run --bin changed -- --help`
- `cargo run --bin changedd -- --help`

Typical local flow today:

- `cargo run --bin changed -- init`
- `cargo run --bin changed -- track /etc/makepkg.conf`
- `cargo run --bin changedd -- --once`
- `cargo run --bin changed -- list -a`

## PACKAGE TRACKING

Package tracking should stay optional and disabled by default.

When added, package events should prefer:

- installs
- removals
- replacements

Routine package updates should remain disabled by default to avoid flooding the
changelog with normal Arch maintenance activity.

## FILES

Current user-scope locations:

- config: `~/.config/changed/config.toml`
- state: `~/.local/state/changed/`
- journal: `~/.local/state/changed/journal.jsonl`
- daemon state: `~/.local/state/changed/daemon-state.json`

Planned system-scope location:

- system state: `/var/lib/changed/`

Environment overrides in the current implementation:

- `CHANGED_CONFIG_HOME`
- `CHANGED_STATE_HOME`

## RETENTION

The local journal currently uses bounded retention with default values:

- `max_events = 10000`
- `max_bytes = 8 MiB`

When these limits are exceeded, the oldest stored events are trimmed.

These limits keep the local journal lightweight today, but they also mean very
old history can roll off over time until archival support exists.

## EXAMPLES

- `changed init`
- `changed track /etc/makepkg.conf`
- `changed track category shell`
- `changed diff enable /etc/makepkg.conf`
- `changed redact enable ~/.config/fish/config.fish`
- `changed list`
- `changed list -a`
- `changed list -C`
- `changed list -t`

Planned next-step examples:

- `changed list -U -C`
- `sudo changed list -SU -a`
- `changed list -i services`
- `changed list -e packages`
- `sudo changed track -S /boot/loader/entries/arch.conf`
- `changed track -U ~/.config/fish/config.fish`

## NOTES

`changed` is meant to preserve a useful memory of tuning and system changes.
It should optimize for readable historical context, not recovery workflows.

See also:

- `README.md`
- `docs/help-text.md`
- `docs/scope-model.md`
- `docs/categories.md`
- `example.md`
