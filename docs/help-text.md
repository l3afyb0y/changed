# Help Text Drafts

These drafts reflect the current CLI surface after the scope-aware refactor.

## `changed --help`

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
  changed init -U
  changed track -U ~/.config/fish/config.fish
  sudo changed track -S /boot/loader/entries/arch.conf
  changed list -U -C
  sudo changed list -SU -a

Run `changed <command> --help` for command-specific help.
```

Development note:

```text
With two binaries in the workspace, local cargo runs should use:
  cargo run --bin changed -- <args>
  cargo run --bin changedd -- <args>
```

## `changed list --help`

```text
changed list - show change history or tracked targets

Usage:
  changed list [options]

Options:
  -S, --system              Use system scope
  -U, --user                Use user scope
  -t, --tracked             Show tracked targets instead of change events
  -i, --include CATEGORY    Include only matching categories
  -e, --exclude CATEGORY    Exclude matching categories
  -p, --path PATH           Filter by exact tracked path
  -a, --all                 Show full retained history
  -s, --since TIME          Show entries since TIME (RFC3339)
  -u, --until TIME          Show entries until TIME (RFC3339)
  -C, --clean-view          Show a low-noise view of relevant changes
  -h, --help                Show this help text
```

Behavior:

```text
With no scope flag, reads default to merged system + current-user output.
`-SU` is a valid explicit merged read.
```

Examples:

```text
changed list
changed list -U
sudo changed list -S
sudo changed list -SU -a -C
changed list -i services
changed list -e packages
changed list -SU -C -i cpu -i gpu -e services
```

## `changed track --help`

```text
changed track - add a tracking target

Usage:
  changed track [scope] <file_path>
  changed track [scope] category <name>
  changed track [scope] package <manager> <package_name>

Scope:
  -S, --system          Track in system scope
  -U, --user            Track in user scope
```

Notes:

```text
Writes must target exactly one scope.
Paths may infer scope automatically when obvious.
Category and package writes should be given an explicit scope.
```

Examples:

```text
changed track -U ~/.config/fish/config.fish
sudo changed track -S /boot/loader/entries/arch.conf
changed track -U category shell
```

## `changed untrack --help`

```text
changed untrack - remove a tracking target

Usage:
  changed untrack [scope] <file_path>
  changed untrack [scope] category <name>
  changed untrack [scope] package <manager> <package_name>

Scope:
  -S, --system          Untrack from system scope
  -U, --user            Untrack from user scope
```

Notes:

```text
Writes must target exactly one scope.
`-SU` is invalid for write operations.
```

## `changed diff --help`

```text
changed diff - control line-diff storage for a tracked path

Usage:
  changed diff [scope] enable <path>
  changed diff [scope] disable <path>
```

## `changed redact --help`

```text
changed redact - control automatic redaction for a tracked path

Usage:
  changed redact [scope] enable <path>
  changed redact [scope] disable <path>
```

## `changed service --help`

```text
changed service - manage the changed systemd service

Usage:
  changed service <action> [scope]

Actions:
  install              Install the changed systemd unit
  start                Start the changed service
  stop                 Stop the changed service
  status               Show service status
```

Notes:

```text
Service commands require an explicit scope.
`install` writes a unit file and runs daemon-reload.
`start` enables and starts the unit for that scope.
`stop` disables and stops the unit for that scope.
```

## `changed daemon --help`

```text
changed daemon - run the tracking daemon in the foreground

Usage:
  changed daemon [options]

Options:
  -S, --system                   Use system scope
  -U, --user                     Use user scope
      --once                     Run one scan cycle and exit
      --interval-seconds N       Polling interval in seconds for continuous mode
  -h, --help                     Show this help text
```

## `changedd --help`

```text
changedd - dedicated daemon for changed

Usage:
  changedd [options]

Options:
      --once                     Run one scan cycle and exit
      --interval-seconds N       Polling interval in seconds for fallback waiting
      --system                   Run in system scope
      --user                     Run in user scope
  -h, --help                     Show this help text
  -V, --version                  Show version
```

Behavior note:

```text
One binary supports both system and user daemon modes.
Later systemd integration should use the same executable for a system service
and an optional per-user service.
```

## `changed init --help`

```text
changed init - initialize changed on this system

Usage:
  changed init [scope]

Behavior:
  Create config and state directories
  Detect host-specific presets for the chosen scope
  Enable default tracking presets
  Print the initial tracking summary
```
