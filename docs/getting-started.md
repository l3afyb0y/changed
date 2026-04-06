# Getting Started

`changed` has two scopes:

- user scope for per-user files like `~/.config`, shell config, and user services
- system scope for machine-wide files like `/etc`, `/boot`, and system units

The default read commands are user-scoped:

- `changed list`
- `changed status`

Use `-SU` for an explicit merged read across system plus current-user scope.

## First Run

For a normal packaged install:

```bash
sudo changed setup
systemctl --user enable --now changedd.service
sudo systemctl enable --now changedd.service
changed status -U
sudo changed status -S
```

If you only want private per-user tracking:

```bash
sudo changed setup
systemctl --user enable --now changedd.service
changed status
changed list
```

`changed setup`:

- writes `/etc/changed/setup.toml`
- scans the preset candidate list
- silently skips missing paths
- updates user and system config with the preset-backed paths that exist
- preserves manual tracked entries
- prints the exact paths it added
- warns per scope if the matching daemon is not running

The current preset-backed default path list is documented in
[default-tracked-paths.md](default-tracked-paths.md).

## First Validation

Make one small harmless edit to a tracked file, save it, then check:

```bash
changed list -U -a
```

For tracked files using unified diffs, `changed` records line-numbered
changed-only hunks, for example:

```text
### 2:08pm
Scope: user
Category: shell
~/.config/fish/config.fish
Changed shell config (+2)
(+)[4] # test 4
(+)[5] # test 5
```

Separate saves stay separate events. Multiple edits in one save become one
event.

## Common Commands

```bash
changed list
changed list -U -a
changed list -P
sudo changed list -S -a
changed status -P
changed track -U ~/.config/fish/config.fish
changed track ~/.config/fish/config.fish -U
sudo changed track -S /etc/makepkg.conf
changed diff -U enable ~/.config/fish/config.fish
changed redact -U enable ~/.config/fish/config.fish
changed history clear -U
sudo changed history clear -SU
```

## Local Development

With two binaries in the workspace, local cargo runs should be explicit:

```bash
cargo run --bin changed -- --help
cargo run --bin changedd -- --help
```

Typical local flow:

```bash
cargo run --bin changed -- init -U
cargo run --bin changed -- init -S
sudo cargo run --bin changed -- setup
cargo run --bin changedd -- --user --once
sudo cargo run --bin changedd -- --system --once
```
