changed(1)

# NAME

changed - lightweight system tuning changelog

# SYNOPSIS

*changed* <command> [options]

Dedicated daemon binary:

*changedd* [options]

# DESCRIPTION

*changed* records a readable history of system tuning and configuration
changes while its daemon is running.

It is built to answer:

- What did I change over time?
- Which changes mattered for CPU, GPU, services, shell, boot, or build tuning?
- What do I need to carry forward to a new install or new hardware?

It is not a backup, rollback, snapshot, or recovery tool.

# STATUS

Current features include:

- scope-aware user and system tracking
- a dedicated daemon binary
- file-backed journaling
- diff and redaction controls
- include/exclude category filtering
- optional pager output for history views
- scoped service install, start, stop, and status commands
- a dedicated *changed status* diagnostics command
- a machine-wide *changed setup* onboarding command

Still planned:

- package event tracking
- service-state event tracking

# SCOPE MODEL

*changed* supports two scopes:

- system scope
- user scope

Use *-S*, *--system* for machine-wide paths such as /etc, bootloader entries,
system units, and scheduler config.

Use *-U*, *--user* for per-user files such as ~/.config, shell startup files,
user services, and desktop or audio config.

For read commands, no scope flag defaults to current-user output. *-SU* is a
valid explicit merged read of system plus current-user history.

For write commands, exactly one scope must usually be targeted. The current
exception is *changed history clear*, which also accepts *-SU* to clear both
scopes in one confirmed operation. If scope cannot be inferred safely, the
command should fail and ask for *-S* or *-U*.

When *changed* is run under sudo, user scope still refers to the invoking
user's config and state directories rather than root's personal user scope.

# COMMANDS

*init*
	Initialize configuration, state directories, and default tracking presets
	for one scope.

*daemon*
	Run the tracking daemon in the foreground for one selected scope.

*service* <action>
	Manage the systemd service.

	Actions:

	- install
	- start
	- stop
	- status

	The *install* action writes a generated scope-specific unit and runs
	daemon-reload. For packaged installs, the unit files are already shipped
	under /usr/lib/systemd, so *install* is mainly for local development or
	non-packaged installs.

	Packaged upgrades do not restart either scope automatically. Restart the
	scope you use explicitly after reinstalling:

	- systemctl --user restart changedd.service
	- sudo systemctl restart changedd.service

*setup*
	Write a shared setup profile once and seed the full preset set for both
	scopes, keeping only the paths that actually exist.

	It requires root, accepts no scope flags, writes
	*/etc/changed/setup.toml*, silently skips missing candidates, preserves
	manual tracked entries, prints the exact paths it tracked, warns per scope
	when the matching daemon is not running, and does not start or restart
	services.

*status*
	Show operational diagnostics for one or both scopes.

	It reports scope paths, shared setup profile state, tracked counts,
	categories, watcher roots, service state, daemon PID when available,
	recent journal/daemon-state metadata, and warnings for obvious operational
	issues.

*track*
	Track a file path, category preset, or package target in one scope.

*untrack*
	Remove a tracked file path, category preset, or package target in one scope.

*list*
	Show change history or tracked targets.

*diff* <action> <path>
	Enable or disable line-diff storage for a tracked path.

*redact* <action> <path>
	Enable or disable automatic redaction for a tracked path.

*history* clear
	Clear stored journal data and the daemon baseline for one or both scopes
	after a destructive confirmation prompt that names the selected scope.

# LIST BEHAVIOR

*changed list* is the primary read command.

By default it shows a recent slice of history for the current user scope.
Use *-SU* for an explicit merged system plus current-user view.

Useful flags:

- *-a*, *--all* to show full retained history
- *-C*, *--clean-view* for a low-noise view
- *--color* auto|always|never to control ANSI color output
- *--pager* to open the rendered output in $PAGER or less -R
- *-i*, *--include* CATEGORY to keep only matching categories
- *-e*, *--exclude* CATEGORY to remove categories from the result

If both include and exclude are present, exclusion wins.

# DAEMON BEHAVIOR

The daemon uses an event-driven watcher with scan and diff verification.

- the first daemon run captures a baseline without emitting synthetic events
- config changes are reloaded automatically
- newly added tracked targets are silently baselined on reload
- existing tracked targets preserve prior observation state across reloads
- watcher events refresh only the affected tracked path or tracked-directory
  descendant instead of rebuilding the whole tracked scope

*changed status* is the preferred command for checking whether a scope is
healthy, configured, and actively recording changes.

# DEVELOPMENT

Typical local commands:

- *cargo run --bin changed -- init -U*
- *sudo cargo run --bin changed -- setup*
- *cargo run --bin changed -- track -U ~/.config/fish/config.fish*
- *cargo run --bin changed -- track ~/.config/fish/config.fish -U*
- *cargo run --bin changed -- status*
- *cargo run --bin changedd -- --user --once*
- *cargo run --bin changed -- list -U -C*

For local development installs, *changed service install* generates a unit file
that points at the current binary path.

# FILES

User scope:

- ~/.config/changed/config.toml
- ~/.local/state/changed/journal.jsonl
- ~/.local/state/changed/daemon-state.json

System scope:

- /etc/changed/config.toml
- /etc/changed/setup.toml
- /var/lib/changed/journal.jsonl
- /var/lib/changed/daemon-state.json

Environment overrides:

- CHANGED_CONFIG_HOME
- CHANGED_STATE_HOME
- CHANGED_SYSTEM_CONFIG_HOME
- CHANGED_SYSTEM_STATE_HOME

These override the config and state roots, so they also control where journal
files are stored. There is no separate CHANGED_LIST_LOCATION because
*changed list* reads from the journal inside the selected scope's state
directory. $PAGER is only used when *changed list --pager* is requested.
