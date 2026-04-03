# Help Text Drafts

This file now serves two purposes:

- capture the help text shape that exists today
- capture the next CLI revision we agreed on before service integration

Where the two differ, this document should say so explicitly instead of
pretending the code already changed.

## Current Implementation

These are the help surfaces the binary effectively exposes today.

### `changed --help`

```text
changed - lightweight system tuning changelog

Usage:
  changed <command> [options]

Commands:
  init       Initialize config, state, and default presets
  daemon     Run the tracking daemon in the foreground
  service    Manage the changed systemd service
  track      Add a tracked file, category, or package target
  untrack    Remove a tracked file, category, or package target
  list       Show change history or tracked targets
  diff       Enable or disable line-diff storage for a path
  redact     Enable or disable redaction for a path
  help       Show help for a command
```

### `changed list --help`

```text
changed list - show change history or tracked targets

Usage:
  changed list [options]

Options:
  -t, --tracked         Show tracked targets instead of change events
  -c, --category NAME   Filter by category
  -p, --path PATH       Filter by exact tracked path
  -a, --all             Show full retained history
  -s, --since TIME      Show entries since TIME (RFC3339)
  -u, --until TIME      Show entries until TIME (RFC3339)
  -C, --clean-view      Show a low-noise view of relevant changes
  -h, --help            Show this help text
```

### `changedd --help`

```text
changedd - dedicated daemon for changed

Usage:
  changedd [options]

Options:
      --once                       Run one scan cycle and exit
      --interval-seconds SECONDS   Polling interval in seconds for fallback waiting
  -h, --help                       Show this help text
  -V, --version                    Show version
```

Development note:

```text
With two binaries in the workspace, local cargo runs should use:
  cargo run --bin changed -- <args>
  cargo run --bin changedd -- <args>
```

## Planned Next CLI Revision

This is the agreed direction before service integration.

### `changed --help`

```text
changed - lightweight system tuning changelog

Usage:
  changed <command> [options]

Commands:
  init       Initialize config, state, and default presets
  daemon     Run the tracking daemon in the foreground
  service    Manage the changed systemd service
  track      Add a tracked file, category, or package target
  untrack    Remove a tracked file, category, or package target
  list       Show change history or tracked targets
  diff       Enable or disable line-diff storage for a path
  redact     Enable or disable redaction for a path
  help       Show help for a command

Examples:
  changed init
  changed track -U ~/.config/fish/config.fish
  sudo changed track -S /boot/loader/entries/arch.conf
  changed list -U -C
  sudo changed list -SU -a
  changed service status

Run `changed <command> --help` for command-specific help.
```

### `changed list --help`

```text
changed list - show change history or tracked targets

Usage:
  changed list [options]

Scope:
  -S, --system          Use system scope
  -U, --user            Use current-user scope

Notes:
  With no scope flag, reads default to merged system + current-user output.
  `-SU` is a valid explicit merged read.

Options:
  -t, --tracked                 Show tracked targets instead of change events
  -i, --include CATEGORY        Include only matching categories
  -e, --exclude CATEGORY        Exclude matching categories
  -p, --path PATH               Filter by exact tracked path
  -a, --all                     Show full retained history
  -s, --since TIME              Show entries since TIME (RFC3339)
  -u, --until TIME              Show entries until TIME (RFC3339)
  -C, --clean-view              Show a low-noise view of relevant changes
  -h, --help                    Show this help text

Examples:
  changed list
  changed list -U
  sudo changed list -S
  sudo changed list -SU -a -C
  changed list -i services
  changed list -e packages
  changed list -SU -C -i cpu -i gpu -e services
  changed list -p /etc/makepkg.conf
```

### `changed track --help`

```text
changed track - add a tracking target

Usage:
  changed track [scope] <file_path>
  changed track [scope] category <name>
  changed track [scope] package <manager> <package_name>

Scope:
  -S, --system          Track in system scope
  -U, --user            Track in user scope

Notes:
  Writes must target exactly one scope.
  If scope is not specified, changed may infer it from the path.
  If scope is unclear, the command should fail.

Examples:
  changed track -U ~/.config/fish/config.fish
  sudo changed track -S /boot/loader/entries/arch.conf
  changed track -U category shell
```

### `changed untrack --help`

```text
changed untrack - remove a tracking target

Usage:
  changed untrack [scope] <file_path>
  changed untrack [scope] category <name>
  changed untrack [scope] package <manager> <package_name>

Scope:
  -S, --system          Untrack from system scope
  -U, --user            Untrack from user scope

Notes:
  Writes must target exactly one scope.
  `-SU` is invalid for write operations.

Examples:
  changed untrack -U ~/.config/fish/config.fish
  sudo changed untrack -S /boot/loader/entries/arch.conf
  changed untrack -U category shell
```

### `changed service --help`

```text
changed service - manage the changed systemd service

Usage:
  changed service <action> [scope]

Actions:
  install              Install the changed systemd unit
  start                Start the changed service
  stop                 Stop the changed service
  status               Show service status

Scope:
  -S, --system         Target the system service
  -U, --user           Target the user service
```

### `changedd --help`

```text
changedd - dedicated daemon for changed

Usage:
  changedd [options]

Options:
      --once                       Run one scan cycle and exit
      --interval-seconds SECONDS   Polling interval in seconds for fallback waiting
      --system                     Run in system scope
      --user                       Run in user scope
  -h, --help                       Show this help text
  -V, --version                    Show version
```

Behavior note:

```text
One binary is expected to support both system and user service modes.
The system service and optional user service should use the same executable with
different scope selection and storage locations.
```
