# changed setup Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a root-only `changed setup` onboarding command that performs one-shot machine probing, writes a shared machine profile, and applies supported hardware/distro-aware tracking defaults without introducing background polling.

**Architecture:** `changed setup` will be a machine-level bootstrap command, not a daemon feature. It will probe once under `sudo`, persist the detected or selected platform profile in a shared config file, and use that profile to seed or update both user and system tracking presets. The daemons will consume the shared profile passively during config/preset evaluation; they will never perform repeated hardware or distro probing themselves.

**Tech Stack:** Rust CLI (`clap`), existing `App`/`AppPaths` architecture, TOML config via `serde`, system inspection via one-shot reads from `/proc`, `/sys`, `/etc/os-release`, and existing config/state rendering and status reporting.

---

## Design Decisions

- `changed setup` is a first-run onboarding command.
- It is intentionally machine-wide and requires root.
- It accepts no scope flags and behaves as an explicit machine bootstrap, not `-S` or `-U`.
- It writes one shared machine profile file, recommended at `/etc/changed/setup.toml`.
- It may also update user and system `config.toml` files with setup-derived preset targets, but the canonical detected machine description lives in the shared setup file.
- Daemons remain event-driven and passive. They read config; they do not probe hardware repeatedly.
- Environment variables are not the primary persistence layer for machine profile selection. They may be added later as override/testing hooks, but not as the main `setup` storage model.

## Scope of v0.5.x Setup

Supported in the first implementation:
- distro detection and persistence
- CPU vendor detection and persistence
- GPU vendor detection and persistence
- shell detection and persistence
- setup status summary output
- setup-aware preset expansion for supported combinations

Explicitly out of scope for the first pass:
- package event hooks
- service-state event hooks
- repeated hardware rescans
- live reconfiguration daemons
- an interactive ncurses/TUI wizard
- fully arbitrary user-defined hardware profiles

## Proposed Shared File

Create `/etc/changed/setup.toml` with a shape like:

```toml
version = 1
distro = "arch"
cpu_vendor = "amd"
gpu_vendors = ["nvidia"]
system_shells = ["fish"]
detected_by = "changed setup"
detected_at = "2026-04-05T22:00:00Z"
```

This file is:
- machine-wide
- root-managed
- readable by both user and system CLI/service flows
- stable enough to use later for package manager and distro-specific event expansion

## CLI Shape

Command:

```text
sudo changed setup
```

Rules:
- no `-S`
- no `-U`
- fail if not root
- allow `--help`
- keep future room for `--debug` or explicit override flags, but do not add them in the first implementation unless needed

Behavior:
- inspect machine profile
- load existing setup file if present
- show what was detected
- write or update shared setup file
- ensure user/system config/state skeletons exist as needed
- apply setup-aware preset expansion to both scopes in a conservative, idempotent way
- print a summary plus next-step guidance

## Data Flow

1. User runs `sudo changed setup`
2. CLI verifies root and rejects scope flags
3. Setup inspector reads:
   - `/etc/os-release`
   - `/proc/cpuinfo`
   - `/sys/class/drm`
   - `/etc/passwd` or shell discovery logic for likely system shells
4. Inspector normalizes detected values into internal enums
5. App writes `/etc/changed/setup.toml`
6. App initializes user/system config if missing
7. Setup-aware preset expansion adds matching tracked paths into user/system configs
8. Command prints summary and recommended next steps:
   - `changed status -U`
   - `sudo changed status -S`
   - restart/start scoped services if desired

## Config/Preset Strategy

Current `detect_presets(scope)` is too static for this feature. Replace or extend it so preset expansion can accept a shared machine profile:

- `detect_presets(scope)` becomes thin wrapper
- new path:
  - load optional shared setup profile
  - expand base presets
  - expand hardware/distro-specific presets when the profile is present

This keeps the machine-specific logic centralized instead of scattering `if AMD`, `if Nvidia`, `if Arch` checks through unrelated code.

## Suggested Internal Types

Add new setup/profile types, likely in a new file:

```rust
pub struct SetupProfile {
    pub version: u32,
    pub distro: Distro,
    pub cpu_vendor: CpuVendor,
    pub gpu_vendors: Vec<GpuVendor>,
    pub system_shells: Vec<ShellKind>,
    pub detected_at: OffsetDateTime,
}
```

With enums:
- `Distro`
- `CpuVendor`
- `GpuVendor`
- `ShellKind`

Each should serialize cleanly to TOML with lowercase snake_case string values.

## Task 1: Add the setup profile data model

**Files:**
- Create: `src/setup.rs`
- Modify: `src/lib.rs`
- Test: `src/setup.rs`

**Step 1: Write failing serialization/deserialization tests**

Cover:
- TOML round-trip for a full `SetupProfile`
- stable string values for distro/vendor enums
- multiple GPU vendor support

**Step 2: Run the targeted test**

Run:

```bash
cargo test setup::
```

Expected: fail because `src/setup.rs` does not exist yet.

**Step 3: Write minimal implementation**

Implement:
- `SetupProfile`
- `Distro`
- `CpuVendor`
- `GpuVendor`
- `ShellKind`
- serde derives and defaults where appropriate

**Step 4: Run tests**

Run:

```bash
cargo test setup::
```

Expected: pass.

## Task 2: Add one-shot system probing helpers

**Files:**
- Create: `src/app/setup.rs`
- Modify: `src/app.rs`
- Test: `src/app/setup.rs`

**Step 1: Write failing detector normalization tests**

Cover:
- parsing `/etc/os-release` snippets into supported distro enum values
- parsing `/proc/cpuinfo` snippets into CPU vendor
- mapping DRM vendor/device hints into GPU vendor(s)
- shell path normalization into `ShellKind`

**Step 2: Run targeted tests**

Run:

```bash
cargo test app::setup::
```

Expected: fail.

**Step 3: Write minimal implementation**

Implement pure parsing helpers first:
- `detect_distro_from_str`
- `detect_cpu_vendor_from_str`
- `detect_gpu_vendors_from_paths` or testable helper
- `detect_shells_from_passwd_or_paths`

Then wrap them in one-shot filesystem readers for real execution.

**Step 4: Run tests**

Run:

```bash
cargo test app::setup::
```

Expected: pass.

## Task 3: Add shared setup profile path handling

**Files:**
- Modify: `src/app/paths.rs`
- Modify: `src/app.rs`
- Test: `src/app.rs` or `src/app/setup.rs`

**Step 1: Write failing path tests**

Cover:
- shared setup file resolves to `/etc/changed/setup.toml`
- user-scope sudo behavior does not change shared machine-profile path semantics

**Step 2: Run targeted tests**

Run:

```bash
cargo test path
```

**Step 3: Implement**

Add helper(s):
- `shared_setup_file() -> PathBuf`
- optionally `shared_setup_dir() -> PathBuf`

Keep this separate from user/system config roots so the machine profile remains clearly machine-scoped.

**Step 4: Run tests**

Run:

```bash
cargo test path
```

## Task 4: Add setup-aware preset expansion

**Files:**
- Modify: `src/app.rs`
- Possibly Create: `src/app/presets.rs`
- Test: `src/app.rs`

**Step 1: Write failing preset tests**

Cover:
- base presets still work with no setup profile
- Arch + Nvidia + AMD CPU adds the expected extra tracked targets
- user and system scopes get only their relevant setup-aware paths
- repeated setup runs are idempotent and do not duplicate tracked paths

**Step 2: Run targeted tests**

Run:

```bash
cargo test preset
```

**Step 3: Implement**

Refactor preset expansion so it can consume:
- `scope`
- optional `SetupProfile`

Keep base presets and hardware/distro-specific presets as separate layers:
- base presets
- distro presets
- CPU presets
- GPU presets
- shell presets

Do not add speculative targets without an explicit supported mapping table.

**Step 4: Run tests**

Run:

```bash
cargo test preset
```

## Task 5: Add `App::setup()` orchestration

**Files:**
- Modify: `src/app.rs`
- Modify: `src/app/setup.rs`
- Test: `src/app.rs`

**Step 1: Write failing orchestration tests**

Cover:
- setup writes shared profile
- setup initializes missing configs
- setup updates both user and system configs conservatively
- setup remains idempotent across repeated runs
- setup summary mentions detected machine profile and resulting tracked counts

**Step 2: Run targeted tests**

Run:

```bash
cargo test setup_writes
```

**Step 3: Implement**

Add:
- `pub fn setup(&self) -> Result<String>`

Behavior:
- require root at CLI layer
- detect and normalize profile
- save profile
- initialize/update both scopes
- return readable summary

**Step 4: Run tests**

Run:

```bash
cargo test setup
```

## Task 6: Add CLI command and help text

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs` if needed
- Test: `src/cli.rs`

**Step 1: Write failing CLI tests**

Cover:
- `changed setup --help` works
- `changed setup -S` fails cleanly
- `changed setup -U` fails cleanly
- command description states it is machine-wide and root-only

**Step 2: Run targeted tests**

Run:

```bash
cargo test cli::tests::setup
```

**Step 3: Implement**

Add:
- `Setup(SetupArgs)` command
- no scope flags
- root check in dispatch
- help text with clear examples and warnings

**Step 4: Run tests**

Run:

```bash
cargo test cli::tests::setup
```

## Task 7: Extend diagnostics to surface setup profile

**Files:**
- Modify: `src/app/status.rs`
- Test: `src/app/status.rs`

**Step 1: Write failing status tests**

Cover:
- status renders shared setup profile when present
- status warns clearly when setup has not been run yet

**Step 2: Run targeted tests**

Run:

```bash
cargo test app::status::
```

**Step 3: Implement**

Add optional status section like:
- Setup profile: present/missing
- Distro: arch
- CPU vendor: amd
- GPU vendors: nvidia
- Shells: fish

**Step 4: Run tests**

Run:

```bash
cargo test app::status::
```

## Task 8: Update docs and manpage

**Files:**
- Modify: `README.md`
- Modify: `docs/help-text.md`
- Modify: `docs/changed.1.md`
- Modify: `docs/changed.1.scd`
- Optionally Modify: `docs/scope-model.md`

**Step 1: Update user-facing docs**

Document:
- what `changed setup` is for
- that it is one-shot and root-only
- that it writes a machine profile shared by both scopes
- that it does not add background polling
- what platforms/hardware are officially supported in the first implementation

**Step 2: Update examples**

Examples should include:

```bash
sudo changed setup
changed status -U
sudo changed status -S
```

**Step 3: Regenerate manpage through package build flow**

Run:

```bash
makepkg --printsrcinfo
makepkg -f
```

## Task 9: Full verification

**Files:**
- No new files

**Step 1: Run unit and CLI tests**

```bash
cargo test
```

**Step 2: Run lint**

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

**Step 3: Run packaging**

```bash
makepkg -f
```

**Step 4: Manual smoke test**

On a real system:

```bash
sudo changed setup
changed status -U
sudo changed status -S
systemctl --user restart changedd.service
sudo systemctl restart changedd.service
```

Confirm:
- shared setup profile exists
- status shows setup profile details
- user/system configs are initialized and seeded correctly
- daemon idle CPU remains low after restart

## Notes for Implementation

- Be conservative about supported hardware/distro mappings. Unsupported combos should be detected honestly and either skipped or reported, never guessed.
- Prefer table-driven preset expansion over nested conditionals.
- Keep setup idempotent.
- Do not bury user-specific shell/home logic inside machine-wide setup logic.
- Avoid env-var-first design for machine profile persistence. Environment overrides can come later if a real testing/debugging need appears.
- Do not auto-start or auto-restart services from `setup` in the first implementation.

## Recommended v1 Support Matrix

Start with a small, honest support matrix:
- Distros: `arch`
- CPU vendors: `amd`, `intel`
- GPU vendors: `nvidia`, `amd`, `intel`
- Shells: `bash`, `fish`, `zsh`

Unsupported detections should be rendered clearly in setup output and status output.

## Recommended Follow-Up After Setup

Once `changed setup` exists, the next logical follow-ons are:
- package event tracking keyed off distro/package manager
- optional `changed config` surfacing setup profile and tunables
- explicit override/edit support for the setup profile if detection is wrong

