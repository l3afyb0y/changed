# Scope Model

This document describes the intended scope and security model for `changed`
before systemd integration lands.

It is a design document for the next CLI revision, not a claim that every part
of this behavior is already implemented.

## Why Scopes Exist

`changed` needs to track both:

- machine-wide tuning such as bootloader entries, kernel cmdline files,
  `/etc` configuration, and system units
- per-user tuning such as `~/.config`, shell startup files, and user services

Those two kinds of data should not live in one flat journal with one flat
permission model.

## Scope Types

### System Scope

System scope is for machine-wide changes that affect the whole system.

Examples:

- `/etc/makepkg.conf`
- `/boot/loader/entries/*.conf`
- `/etc/systemd/system/*.service`
- `/etc/sysctl.d/*.conf`

### User Scope

User scope is for per-user configuration and tuning.

Examples:

- `~/.config/fish/config.fish`
- `~/.config/systemd/user/*.service`
- `~/.bashrc`
- `~/.config/pipewire/*.conf`

## CLI Scope Flags

The intended scope flags are:

- `-S, --system`
- `-U, --user`

### Read Commands

For read-oriented commands such as `changed list`:

- no scope flag should mean merged default view
- `-S` should mean system scope only
- `-U` should mean current-user scope only
- `-SU` should mean explicit merged view of system plus current user

Examples:

- `changed list`
- `changed list -S`
- `changed list -U`
- `changed list -SU -a -C`

### Write Commands

For write-oriented commands such as `track` and `untrack`:

- exactly one scope should be targeted
- `-SU` should be invalid
- when no scope flag is provided, scope may be inferred only if the path is
  clearly user or clearly system
- if scope is not obvious, the command should fail

Expected error shape:

```text
Error: unclear scope. Please specify -S or -U.
```

Examples:

- `changed track -S /boot/loader/entries/arch.conf`
- `changed track -U ~/.config/fish/config.fish`

## Category Filters

The current design direction is to replace `--category` with symmetric include
and exclude filters:

- `-i, --include <CATEGORY>`
- `-e, --exclude <CATEGORY>`

They should be repeatable.

Examples:

- `changed list -i services`
- `changed list -e packages`
- `changed list -SU -C -i cpu -i gpu -e services`

Filter rules:

- `--include` means only these categories
- `--exclude` removes categories from the result
- if both are present, exclusion wins

## Security Model

The goal is not to force every `changed` command through `sudo`.

Instead, data should be private to its scope owner:

- system logs and state should be root-owned and root-readable only
- user logs, state, and config should be owned by the user and created with
  private permissions
- reading system-scope logs should require privilege
- reading another user's logs should require privilege and an explicit future
  user-selection mechanism

This is meant to reduce accidental exposure without breaking normal user-scope
tracking workflows.

## Services

The long-term service model is:

- one system service using `changedd`
- one optional user service per user, also using `changedd`

The intended enable flows are separate:

- `sudo systemctl enable --now changedd`
- `systemctl --user enable --now changedd`

That keeps user tracking opt-in and avoids surprising users with automatic
per-user background services.

## Open Questions

These are the main design questions still worth revisiting before service work:

- should the merged default read view always include both system and current
  user scopes
- how should explicit current-user selection look if we later add root-only
  inspection of other users' logs
- whether user-level logs should eventually support optional encryption at rest
