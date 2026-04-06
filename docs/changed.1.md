# CHANGED(1)

## NAME

`changed` - lightweight system tuning changelog for systemd-based Linux systems.

## SYNOPSIS

`changed <command> [options]`

Dedicated daemon binary:

`changedd [options]`

## DESCRIPTION

`changed` records a readable history of system tuning and configuration
changes while its daemon is running.

It is designed to answer:

- What did I change over time?
- Which changes were related to CPU, GPU, services, shell, boot, or build tuning?
- What do I need to carry forward to a new install or new hardware?

`changed` is not a backup, rollback, snapshot, or recovery tool.

## STATUS

The current codebase provides:

- a working CLI
- a dedicated daemon binary
- user and system scoped config/state roots
- file-backed journaling
- diff and redaction controls
- category-aware rendering with include/exclude filters
- optional pager output for history views
- scoped systemd service install/start/stop/status commands
- a dedicated `changed status` diagnostics command
- a machine-wide `changed setup` onboarding command

Package tracking, service-state event tracking, CLI config management, and
setup refinement are still future work.

## SCOPE MODEL

`changed` works with two scopes:

- system scope
- user scope

System scope is for machine-wide tuning such as:

- `/etc`
- `/boot/loader/entries`
- system units
- boot and scheduler config

User scope is for per-user tuning such as:

- `~/.config`
- shell startup files
- user services
- user audio or desktop tuning

The scope flags are:

- `-S`, `--system`
- `-U`, `--user`

For read commands, no scope flag defaults to current-user output. `-SU` is a
valid explicit merged read of system plus current-user history.

For write commands, exactly one scope must usually be targeted. The current
exception is `changed history clear`, which also accepts `-SU` to clear both
scopes in one confirmed operation.

When `changed` is run under `sudo`, user scope continues to refer to the
invoking user's config and state directories rather than root's personal user
scope.

See also [docs/scope-model.md](scope-model.md).

## COMMANDS

### `init`

Initialize configuration, state directories, and default tracking presets for a
single scope.

### `daemon`

Run the tracking daemon in the foreground for one selected scope.

Supported today:

- `-S`, `--system`
- `-U`, `--user`
- `--once`

For long-running daemon use, prefer the dedicated binary `changedd`.

### `service <action>`

Manage the systemd service.

Supported actions:

- `install`
- `start`
- `stop`
- `status`

Service commands require an explicit scope.

Current behavior:

- `install` writes a generated scope-specific unit file for local/dev or non-packaged installs
- `start` runs `enable --now` for that scope
- `stop` runs `disable --now` for that scope
- `status` runs `systemctl status` for that scope

For packaged installs, the unit files are already shipped under `/usr/lib/systemd`.
Packaged upgrades do not restart either scope automatically.

### `setup`

Write a shared setup profile once and seed preset-backed tracked paths for both
scopes, keeping only the paths that actually exist.

Current behavior:

- requires root
- accepts no scope flags
- writes `/etc/changed/setup.toml`
- scans preset candidate paths and silently skips missing files
- updates preset-backed tracked paths in both user and system config
- preserves manual tracked entries
- prints the exact user/system paths it successfully tracked
- warns per scope when the matching daemon is not currently running
- does not start or restart services
- does not add background polling

### `status`

Show operational diagnostics for one or both scopes.

Supported today:

- `-S`, `--system`
- `-U`, `--user`
- `-P`, `--pager`

The command reports:

- whether the scope is initialized
- shared setup profile path and detected hardware when present
- config, state, journal, and daemon-state paths
- tracked path/package counts and tracked categories
- watcher roots derived from the current config
- service active/enabled state and daemon PID when available
- last recorded event time and daemon-state update time
- warnings for obvious operational issues

### `track`

Track a file path, category preset, or package target in one scope.

Examples:

- `changed track -U ~/.config/fish/config.fish`
- `changed track ~/.config/fish/config.fish -U`
- `sudo changed track -S /boot/loader/entries/arch.conf`
- `changed track -U category shell`

### `untrack`

Remove a tracked file path, category preset, or package target in one scope.

### `list`

Show the changelog or tracked targets.

Supported today:

- `-S`, `--system`
- `-U`, `--user`
- `-t`, `--tracked`
- `-i`, `--include <category>`
- `-e`, `--exclude <category>`
- `-p`, `--path <file_path>`
- `-a`, `--all`
- `-s`, `--since <time>`
- `-u`, `--until <time>`
- `-C`, `--clean-view`
- `-P`, `--pager`

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

### `history clear`

Clear stored journal data and the daemon baseline for one or both scopes.

This command is destructive and prompts before removing files. The prompt names
the selected scope explicitly as `user`, `system`, or `user and system`.

## LIST BEHAVIOR

`changed list` is the primary read command.

Default output should:

- show a recent slice of history
- group entries in a readable changelog style
- default to current-user scope when no scope flags are given

`changed list -a` shows the full retained history.

`changed list -SU` shows merged system plus current-user history.

`changed list -C` provides a low-noise day-to-day reading mode.

`changed list -P`, `changed list --pager`, `changed status -P`, and
`changed status --pager` open the rendered output in the standard `$PAGER`
command when set, or in `less -R` otherwise.

`--clean-view` changes presentation only. It does not delete data, alter
history, or change what the daemon stores.

Category filter semantics:

- `-i`, `--include` means only these categories
- `-e`, `--exclude` removes categories from the result
- repeated filters are allowed
- exclusion wins if both are present

For tracked files using unified diffs, history entries show only changed lines
with line numbers. Unchanged context lines are currently omitted.

## DAEMON BEHAVIOR

The daemon uses an event-driven watcher with scan and diff verification.

- the first daemon run captures a baseline without emitting synthetic events
- config changes are reloaded automatically
- newly added tracked targets are silently baselined on reload
- existing tracked targets preserve prior observation state across reloads
- watcher events refresh only the affected tracked path or tracked-directory
  descendant instead of rebuilding the whole tracked scope

`changed status` is the preferred command for checking whether a scope is
healthy, configured, and actively recording changes.

## DEVELOPMENT

With two binaries in the workspace, local cargo runs should be explicit:

- `cargo run --bin changed -- --help`
- `cargo run --bin changedd -- --help`

Typical local flow:

- `cargo run --bin changed -- init -U`
- `cargo run --bin changed -- init -S`
- `sudo cargo run --bin changed -- setup`
- `cargo run --bin changed -- track -U ~/.config/fish/config.fish`
- `cargo run --bin changed -- track ~/.config/fish/config.fish -U`
- `cargo run --bin changed -- status`
- `cargo run --bin changedd -- --user --once`
- `cargo run --bin changed -- list -U -C`

## FILES

User scope:

- config: `~/.config/changed/config.toml`
- state: `~/.local/state/changed/`
- journal: `~/.local/state/changed/journal.jsonl`
- daemon state: `~/.local/state/changed/daemon-state.json`

System scope:

- config: `/etc/changed/config.toml`
- setup profile: `/etc/changed/setup.toml`
- state: `/var/lib/changed/`
- journal: `/var/lib/changed/journal.jsonl`
- daemon state: `/var/lib/changed/daemon-state.json`

Environment overrides:

- `CHANGED_CONFIG_HOME`
- `CHANGED_STATE_HOME`
- `CHANGED_SYSTEM_CONFIG_HOME`
- `CHANGED_SYSTEM_STATE_HOME`

These override the config and state roots, so they also control where journal
files are stored.

## RETENTION

The local journal currently uses bounded retention with default values:

- `max_events = 10000`
- `max_bytes = 8 MiB`

When these limits are exceeded, the oldest stored events are trimmed.

## EXAMPLES

- `changed list`
- `changed list -U`
- `sudo changed list -S`
- `sudo changed list -SU -a -C`
- `changed status`
- `sudo changed setup`
- `sudo changed status -SU`
- `changed list -i services`
- `changed list -e packages`
- `sudo changed track -S /boot/loader/entries/arch.conf`
- `changed track -U ~/.config/fish/config.fish`
- `changed track ~/.config/fish/config.fish -U`
- `changed diff -U enable ~/.config/fish/config.fish`
- `sudo changed redact -S disable /etc/makepkg.conf`
- `changed history clear -U`
- `sudo changed history clear -SU`
- `changed service -U install`
- `changed service -U start`
- `sudo changed service -S install`
- `sudo changed service -S status`

## NOTES

`changed` should optimize for readable historical context, not recovery
workflows.

See also:

- `README.md`
- `docs/getting-started.md`
- `docs/operations.md`
- `docs/help-text.md`
- `docs/scope-model.md`
- `docs/categories.md`
- `example-log.md`
