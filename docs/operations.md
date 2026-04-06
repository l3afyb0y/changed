# Operations And Behavior

This document collects the runtime behavior that is useful once `changed` is
already installed and in use.

## Daemon Behavior

`changedd` is event-driven. It blocks on filesystem notifications and refreshes
only the affected tracked path or tracked-directory descendant instead of
rebuilding the entire tracked scope on every wake.

Current behavior:

- first scan captures a baseline without emitting synthetic events
- newly added tracked targets are silently baselined on reload
- config changes are reloaded automatically
- existing tracked targets preserve prior observation state across reloads

## Journal Behavior

The journal is append-only within the current retention window.

- `changed list` shows a recent slice by default
- `changed list -a` shows the full retained journal
- `changed list -C` changes presentation only
- `changed list -P`, `changed list --pager`, `changed status -P`, and
  `changed status --pager` open the rendered output in the standard `$PAGER`
  command or `less -R`
- `changed history clear -U`, `-S`, and `-SU` delete the journal and daemon baseline for the selected scope or scopes

## Diff Behavior

For tracked files using unified diffs:

- only changed lines are shown
- line numbers are included
- unchanged context lines are currently omitted
- separate saves remain separate events
- multiple edits in one save become one event

This keeps the history readable without dumping entire files for small edits.

## Setup

`changed setup` is the machine-wide onboarding command.

```bash
sudo changed setup
```

It:

- writes `/etc/changed/setup.toml`
- scans the preset candidate list
- keeps only the paths that exist
- updates both user and system config
- preserves manual tracked entries
- prints the exact paths it tracked
- warns per scope if the corresponding daemon is not running
- does not start or restart services

## Status

`changed status` is the operational diagnostics command.

It reports:

- initialization state
- setup profile state
- config, state, journal, and daemon-state paths
- tracked path and package counts
- categories present
- watcher roots
- service active/enabled state and daemon PID when available
- recent journal and daemon-state timestamps
- warnings for obvious operational issues

## Service Management

Packaged installs already ship the unit files under `/usr/lib/systemd`.

Enable them directly with `systemctl`:

```bash
systemctl --user enable --now changedd.service
sudo systemctl enable --now changedd.service
```

`changed service install` is mainly for local development or non-packaged
installs where the unit should be generated from the current binary path.

Packaged upgrades do not restart either scope automatically:

```bash
systemctl --user restart changedd.service
sudo systemctl restart changedd.service
```

## Files

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

These override the config and state roots, which also changes where the journal
files live.

## Default Presets

The current preset-backed path list lives in
[default-tracked-paths.md](default-tracked-paths.md).

## Safety Model

Tracking, diffing, and redaction are separate controls:

- tracking decides whether a target is watched
- diff decides whether readable line changes are stored
- redaction decides whether stored diffs mask likely sensitive values

The intended permission model is:

- system config and state remain root-owned
- user config and state remain private to the owning user
- user-scope workflows do not require `sudo` by default
- shell-like files stay conservative by default

Redaction is heuristic, not perfect. Highly sensitive files should still be
handled carefully.
