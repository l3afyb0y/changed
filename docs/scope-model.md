# Scope Model

This document describes the current scope and security model for `changed`.

It is the foundation for the current scoped CLI and `systemd` service model.

## Why Scopes Exist

`changed` needs to track both:

- machine-wide tuning such as bootloader entries, kernel cmdline files, `/etc` configuration, and system units
- per-user tuning such as `~/.config`, shell startup files, and user services

Those two kinds of data should not live in one flat journal with one flat permission model.

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

The scope flags are:

- `-S, --system`
- `-U, --user`

### Read Commands

For read-oriented commands such as `changed list`:

- no scope flag means merged default view
- `-S` means system scope only
- `-U` means current-user scope only
- `-SU` is an explicit merged view of system plus current user
- requesting system scope without privilege should fail clearly and suggest
  re-running with `sudo` or using `-U`

Examples:

- `changed list`
- `changed list -S`
- `changed list -U`
- `changed list -SU -a -C`

### Write Commands

For write-oriented commands such as `track`, `untrack`, `diff`, and `redact`:

- exactly one scope must be targeted
- `-SU` is invalid
- when no scope flag is provided, scope may be inferred only if the path is
  clearly user or clearly system
- if scope is not obvious, the command fails

Expected error shape:

```text
Error: unclear scope. Please specify -S or -U.
```

Examples:

- `changed track -S /boot/loader/entries/arch.conf`
- `changed track -U ~/.config/fish/config.fish`

For category and package writes, scope should be given explicitly.

## Category Filters

Category filtering uses symmetric include and exclude flags:

- `-i, --include <CATEGORY>`
- `-e, --exclude <CATEGORY>`

They are repeatable.

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
- user logs, state, and config should be owned by the user and created with private permissions
- reading system-scope logs generally requires privilege
- reading another user's logs should require privilege and an explicit future user-selection mechanism

This reduces accidental exposure without breaking normal user-scope tracking workflows.

## Services

The current service model is:

- one system service using `changedd --system`
- one optional user service per user using `changedd --user`

The intended enable flows are separate:

- `sudo systemctl enable --now changedd`
- `systemctl --user enable --now changedd`

That keeps user tracking opt-in and avoids surprising users with automatic per-user background services.

## Remaining Questions

These are still worth revisiting as the service model matures:

- how to present merged reads when system scope exists but the current user
  lacks permission to read it
- how explicit user selection should look if we later add root-only inspection
  of other users' logs
- whether user-level logs should eventually support optional encryption at rest
