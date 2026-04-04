# Packaging Workflow

This project uses a hand-maintained `PKGBUILD`.

`cargo pkgbuild` is still useful here, but only as a bootstrap and comparison
tool. It does not know enough about this package's final Arch layout.

## Why `PKGBUILD` Is Hand-Maintained

`changed` packages more than one Rust binary:

- `changed`
- `changedd`

It also installs:

- system and user `systemd` units
- project documentation
- the license file

The generated `PKGBUILD` from `cargo pkgbuild` is a decent Rust-package
starting point, but for this repo it drops package-specific details like the
second binary and packaged unit files.

## Recommended Workflow

For this project:

1. Keep `Cargo.toml` accurate for version, description, license, and homepage.
2. Treat `PKGBUILD` as the Arch-specific source of truth.
3. Keep Arch-specific install paths and package contents in `PKGBUILD`.
4. Use `cargo pkgbuild` only to compare boilerplate changes when starting a new
   package or when reevaluating Rust packaging defaults.
5. Verify with `makepkg` tooling before publishing changes.

## Suggested Loop

When starting a new Rust project:

1. Write normal crate metadata in `Cargo.toml`.
2. Run `cargo pkgbuild` once to get a baseline `PKGBUILD`.
3. Edit that `PKGBUILD` into the real package definition.
4. Add repo-specific assets such as units, completions, man pages, examples,
   docs, or config files.
5. From then on, maintain `PKGBUILD` directly.

For ongoing work on this repo:

1. Update `Cargo.toml` version when releasing.
2. Update `pkgver` and any package install logic in `PKGBUILD`.
3. Build and test the Rust project:

   ```bash
   cargo build --release
   cargo test
   ```

4. Validate packaging metadata:

   ```bash
   makepkg --printsrcinfo > .SRCINFO
   ```

5. When you want a full package check, build it locally:

   ```bash
   makepkg -f
   ```

For packaged installs, the package already provides the `systemd` unit files
under `/usr/lib/systemd/system/` and `/usr/lib/systemd/user/`. That means:

- use `systemctl enable --now changedd.service` for the system service
- use `systemctl --user enable --now changedd.service` for the user service
- reserve `changed service install` for local/dev installs that are not using
  the packaged unit files

## Repo-Friendly Principles

If the long-term goal is official Arch packaging, prefer these defaults:

- build from source
- avoid AUR-specific `-bin` assumptions
- keep runtime dependencies explicit
- install all shipped artifacts intentionally
- only advertise architectures you have actually validated
- keep Arch-specific logic in `PKGBUILD`, not hidden in release automation

That keeps the package easier to review, easier to maintain, and closer to what
an Arch packager would expect.
