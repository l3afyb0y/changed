# Help Text Drafts

These drafts reflect the current CLI surface in `0.5.9`.

## `changed --help`

```text
Lightweight system tuning changelog

Usage: changed <command> [options]

Commands:
  init     Initialize config, state, and default presets
  daemon   Run the tracking daemon in the foreground
  service  Manage the changed systemd service
  setup    Seed the full preset set and keep paths that exist
  status   Show operational diagnostics for changed
  history  Manage recorded history data
  track    Add a tracked file, category, or package target
  untrack  Remove a tracked file, category, or package target
  list     Show change history or tracked targets
  diff     Enable or disable line-diff storage for a path
  redact   Enable or disable redaction for a path
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

Examples:
  changed init
  sudo changed setup
  changed track -U ~/.config/fish/config.fish
  sudo changed track -S /boot/loader/entries/arch.conf
  changed status
  changed list -C
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
Show change history or tracked targets

Usage: changed list [OPTIONS]

Options:
  -S, --system              Use system scope
  -U, --user                Use user scope
  -t, --tracked             Show tracked targets instead of change events
  -i, --include <CATEGORY>  Include only matching categories [possible values: cpu, gpu, services,
                            scheduler, shell, build, boot, audio, packages]
  -e, --exclude <CATEGORY>  Exclude matching categories [possible values: cpu, gpu, services,
                            scheduler, shell, build, boot, audio, packages]
  -p, --path <PATH>         Filter by exact tracked path
  -a, --all                 Show full retained history
  -s, --since <TIME>        Show entries since TIME (RFC3339)
  -u, --until <TIME>        Show entries until TIME (RFC3339)
  -C, --clean-view          Show a low-noise view of relevant changes
      --color <COLOR>       Control color output [default: auto] [possible values: auto, always,
                            never]
  -P, --pager               Open output in the standard $PAGER command (or less -R) instead of printing directly
  -h, --help                Print help

Notes:
  With no scope flags, `changed list` defaults to user scope.

Examples:
  changed list
  changed list -U
  sudo changed list -S
  sudo changed list -SU -a -C
  changed list -i services
  changed list -e packages
  changed list -SU -C -i cpu -i gpu -e services
```

## `changed status --help`

```text
Show operational diagnostics for changed

Usage: changed status [OPTIONS]

Options:
  -S, --system  Use system scope
  -U, --user    Use user scope
  -P, --pager   Open output in the standard $PAGER command (or less -R) instead of printing directly
  -h, --help    Print help

Notes:
  With no scope flags, `changed status` defaults to user scope.
  Use `-SU` for a merged status view across both scopes.

Examples:
  changed status
  changed status -U
  sudo changed status -S
  sudo changed status -SU
```

## `changed setup --help`

```text
Seed the full preset set and keep paths that exist

Usage: changed setup

Options:
  -h, --help  Print help

Notes:
  `changed setup` is a machine-wide onboarding command.
  It requires sudo, accepts no scope flags, writes a shared setup profile,
  scans preset candidate paths, and updates both user and system config
  with the paths that actually exist.
  Missing candidates are skipped silently, and the command prints what landed.

Examples:
  sudo changed setup
```

## `changed history --help`

```text
Manage recorded history data

Usage: changed history <COMMAND>

Commands:
  clear  Clear stored journal data for one or both scopes
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help

Examples:
  changed history clear -U
  sudo changed history clear -S
  sudo changed history clear -SU
```

## `changed history clear --help`

```text
Clear stored journal data for one or both scopes

Usage: changed history clear [OPTIONS]

Options:
  -S, --system  Use system scope
  -U, --user    Use user scope
  -h, --help    Print help

Examples:
  changed history clear -U
  sudo changed history clear -S
  sudo changed history clear -SU
```

## `changed track --help`

```text
Add a tracked file, category, or package target

Usage: changed track [scope] <file_path>
       changed track [scope] category <name>
       changed track [scope] package <manager> <package_name>

Options:
  -S, --system  Use system scope
  -U, --user    Use user scope
  -h, --help    Print help

Scope:
  -S, --system          Track in system scope
  -U, --user            Track in user scope

Notes:
  Writes must target exactly one scope.
  Paths may infer scope automatically when obvious.

Examples:
  changed track -U ~/.config/fish/config.fish
  changed track ~/.config/fish/config.fish -U
  sudo changed track -S /boot/loader/entries/arch.conf
  changed track -U category shell
```

## `changed untrack --help`

```text
Remove a tracked file, category, or package target

Usage: changed untrack [scope] <file_path>
       changed untrack [scope] category <name>
       changed untrack [scope] package <manager> <package_name>

Options:
  -S, --system  Use system scope
  -U, --user    Use user scope
  -h, --help    Print help
```

Notes:

```text
Writes must target exactly one scope.
`-SU` remains invalid here even though `changed history clear` accepts it.
Scope flags are accepted before or after the target path.
```

## `changed diff --help`

```text
Enable or disable line-diff storage for a path

Usage: changed diff [scope] enable <path>
       changed diff [scope] disable <path>
```

## `changed redact --help`

```text
Enable or disable redaction for a path

Usage: changed redact [scope] enable <path>
       changed redact [scope] disable <path>
```

## `changed service --help`

```text
Manage the changed systemd service

Usage: changed service [OPTIONS] <ACTION>

Arguments:
  <ACTION>  [possible values: install, start, stop, status]

Options:
  -S, --system  Use system scope
  -U, --user    Use user scope
  -h, --help    Print help

Notes:
  Service commands require an explicit scope.
  `install` writes a generated unit for local/dev or non-packaged installs.
  For packaged installs, use `systemctl enable --now changedd.service` or
  `systemctl --user enable --now changedd.service` directly.

Examples:
  changed service install -U
  changed service start -U
  sudo changed service install -S
  sudo changed service status -S
```

## `changed daemon --help`

```text
Run the tracking daemon in the foreground

Usage: changed daemon [OPTIONS]

Options:
  -S, --system  Use system scope
  -U, --user    Use user scope
      --once    Run one scan cycle and exit
  -h, --help    Print help

Examples:
  changed daemon -U
  sudo changed daemon -S --once
```

## `changedd --help`

```text
changedd - dedicated daemon for changed

Usage:
  changedd [options]

Options:
  --once                     Run one scan cycle and exit
  --system                   Run in system scope
  --user                     Run in user scope
  -h, --help                 Show this help text
  -V, --version              Show version

Examples:
  changedd --user
  changedd --system --once
```

Behavior note:

```text
One binary supports both system and user daemon modes.
The daemon blocks on watcher events and refreshes only the affected tracked
path or directory descendant instead of rebuilding the entire tracked scope
on every wake.
```

## `changed init --help`

```text
Initialize config, state, and default presets

Usage: changed init [OPTIONS]

Behavior:
  Create config and state directories
  Load setup-aware presets when /etc/changed/setup.toml exists
  Enable default tracking presets
  Print the initial tracking summary
```
