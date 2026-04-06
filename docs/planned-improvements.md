# planned improvements

This is the public roadmap for `changed`.

The core config-change journaling feature is in place. Everything listed here
is future expansion, polish, or workflow refinement.

- Package event tracking
  - package installs
  - removals
  - replacements
  - likely distro-specific manager integration
- Service-state event tracking
  - started, stopped, failed, and restarted events
  - correlate config changes with service behavior
- `changed config`
  - list and edit tunable settings from the CLI
  - retention, diff, redaction, and output defaults
  - safe config edits without manual TOML editing
- Setup and preset refinement
  - clearer preset families
  - setup-aware distro support expansion
  - better control over broad preset directories
- Optional richer diff views
  - context lines or alternate render modes
  - keep the default history output compact
