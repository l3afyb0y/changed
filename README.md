# changed

`changed` is a lightweight system tuning changelog for Arch Linux.

It is built to answer:

- What did I change over time?
- Which changes mattered for CPU, GPU, services, shell, boot, or build tuning?
- What do I need to carry forward to a new install or new hardware?

It is not a backup, rollback, or recovery tool.

## Current Status

The core tracking loop exists today and is usable for local development:

- `changed` manages config, tracked targets, history views, and policy
- `changedd` captures a baseline, watches tracked paths, and appends journal
  events

The project is not yet integrated with systemd. The `changed service ...`
command family is still reserved placeholder surface for that work.

This repository currently has two layers of documentation:

- current implementation notes for what the code already does
- next-step CLI and service design notes for the changes we agreed on before
  systemd integration

That split is intentional. The CLI scope model is evolving, and the docs should
capture the design decisions before we wire them into code.

## Current Shape

The project currently has two binaries:

- `changed`: user-facing CLI for tracking targets, browsing history, and
  managing diff/redaction policy
- `changedd`: dedicated daemon process for watching tracked paths and
  appending journal events

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
cargo run --bin changed -- init
cargo run --bin changed -- track /etc/makepkg.conf
cargo run --bin changed -- track category shell
cargo run --bin changedd -- --once
cargo run --bin changed -- list -t
cargo run --bin changed -- list -a
cargo run --bin changed -- list -C
```

## Project Direction

The project is moving toward two scopes of tracking:

- system scope for machine-wide tuning such as `/etc`, `/boot`, loader entries,
  systemd units, and kernel cmdline files
- user scope for per-user files such as `~/.config`, shell config, and
  user-level services

This is meant to support:

- a system log visible to privileged users
- one private user log per user
- separate system and user services later, even though they will share the same
  `changedd` binary

See [docs/scope-model.md](docs/scope-model.md) for the detailed design.

## What Works Today

- Track exact files
- Track preset categories like `shell`, `build`, or `services`
- Enable or disable diff storage per path
- Enable or disable redaction per path
- Auto-reload daemon config when tracked targets change
- Append a structured journal and render it as a readable changelog
- Show a low-noise daily view with `changed list -C`

## What Does Not Exist Yet

- systemd unit installation and service lifecycle management
- package event tracking
- service state event tracking
- a dedicated `status` command for daemon diagnostics

The `changed service ...` CLI surface is still placeholder-only until systemd
integration lands.

## Planned Next CLI Revision

Before systemd integration, the next CLI pass is expected to add:

- `-S, --system` for system-scope reads and writes
- `-U, --user` for user-scope reads and writes
- composable read scopes like `-SU`
- merged default reads when no scope flag is provided
- `-i, --include <CATEGORY>` for category inclusion filters
- `-e, --exclude <CATEGORY>` for category exclusion filters

For write operations such as `track` and `untrack`:

- exactly one scope should be targeted
- path-based auto-detection may be used when the scope is obvious
- if scope is unclear, the command should fail and ask the user to specify
  `-S` or `-U`

These flags are part of the current design direction, but they are not all
implemented yet.

## Journal Behavior

- The first daemon scan captures a baseline without emitting synthetic events.
- New targets added while the daemon is running are auto-reloaded and silently
  baselined.
- The journal keeps historical changes instead of replacing old entries with
  newer ones.
- `changed list` shows a recent view by default. Use `changed list -a` for the
  full retained journal.
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

- Config: `~/.config/changed/config.toml`
- State: `~/.local/state/changed/`
- Journal: `~/.local/state/changed/journal.jsonl`
- Daemon state: `~/.local/state/changed/daemon-state.json`

Environment overrides:

- `CHANGED_CONFIG_HOME`
- `CHANGED_STATE_HOME`

## Example Output

For the intended human-readable changelog style, see [example.md](example.md).

## Documentation

- [Scope and security model](docs/scope-model.md)
- [CLI help drafts](docs/help-text.md)
- [Man-page-style reference](docs/changed.1.md)
- [Category definitions](docs/categories.md)
