use anyhow::{Context, Result, anyhow};
use changed::app::{App, DaemonOptions, HistoryQuery};
use changed::category::Category;
use changed::config::{DiffMode, RedactionMode};
use changed::scope::Scope;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueHint};
use nix::unistd::Uid;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let app = App::new()?;

    match cli.command {
        Some(Commands::Init(args)) => {
            let scope = args.scope_flags.resolve_single_or_default(Scope::User)?;
            ensure_privileged_scope(scope)?;
            println!("{}", app.init(scope)?);
        }
        Some(Commands::Daemon(args)) => {
            let scope = args.scope_flags.resolve_single_or_default(Scope::User)?;
            ensure_privileged_scope(scope)?;
            let options = DaemonOptions { once: args.once };
            println!("{}", app.run_daemon(scope, options)?);
        }
        Some(Commands::Service(args)) => {
            let scope = args.scope_flags.resolve_single_with_message(
                "Service commands require an explicit scope. Please specify -S or -U.",
            )?;
            ensure_privileged_scope(scope)?;
            println!("{}", app.service_action(args.action.as_str(), scope)?);
        }
        Some(Commands::Status(args)) => {
            let scopes = args.scope_flags.resolve_read_scopes();
            ensure_privileged_scopes(&scopes)?;
            let output = app.status_report(&scopes)?;
            emit_output(&output, args.pager)?;
        }
        Some(Commands::History(args)) => match args.command {
            HistoryCommand::Clear(scope_args) => {
                let scopes = scope_args.scope_flags.resolve_history_clear_scopes()?;
                ensure_privileged_scopes(&scopes)?;
                if confirm_history_clear(&scopes)? {
                    let mut messages = Vec::new();
                    for scope in scopes {
                        messages.push(app.clear_history(scope)?);
                    }
                    println!("{}", messages.join("\n"));
                } else {
                    println!("Aborted.");
                }
            }
        },
        Some(Commands::Track(args)) => match args.mode() {
            TrackMode::File(path) => {
                let scope = resolve_write_scope(&app, args.scope_flags, Some(&path))?;
                ensure_privileged_scope(scope)?;
                println!("{}", app.track_file(scope, &path)?);
            }
            TrackMode::Category(category) => {
                let scope = args.scope_flags.resolve_single_with_message(
                    "Tracking a category requires an explicit scope. Please specify -S or -U.",
                )?;
                ensure_privileged_scope(scope)?;
                println!("{}", app.track_category(scope, category)?);
            }
            TrackMode::Package {
                manager,
                package_name,
            } => {
                let scope = args.scope_flags.resolve_single_with_message(
                    "Tracking a package requires an explicit scope. Please specify -S or -U.",
                )?;
                ensure_privileged_scope(scope)?;
                println!("{}", app.track_package(scope, &manager, &package_name)?);
            }
            TrackMode::Invalid => {
                eprintln!("Invalid track usage. Run `changed track --help` for details.");
                std::process::exit(2);
            }
        },
        Some(Commands::Untrack(args)) => match args.mode() {
            UntrackMode::File(path) => {
                let scope = resolve_write_scope(&app, args.scope_flags, Some(&path))?;
                ensure_privileged_scope(scope)?;
                println!("{}", app.untrack_file(scope, &path)?);
            }
            UntrackMode::Category(category) => {
                let scope = args.scope_flags.resolve_single_with_message(
                    "Untracking a category requires an explicit scope. Please specify -S or -U.",
                )?;
                ensure_privileged_scope(scope)?;
                println!("{}", app.untrack_category(scope, category)?);
            }
            UntrackMode::Package {
                manager,
                package_name,
            } => {
                let scope = args.scope_flags.resolve_single_with_message(
                    "Untracking a package requires an explicit scope. Please specify -S or -U.",
                )?;
                ensure_privileged_scope(scope)?;
                println!("{}", app.untrack_package(scope, &manager, &package_name)?);
            }
            UntrackMode::Invalid => {
                eprintln!("Invalid untrack usage. Run `changed untrack --help` for details.");
                std::process::exit(2);
            }
        },
        Some(Commands::List(args)) => {
            let scopes = args.scope_flags.resolve_read_scopes();
            ensure_privileged_scopes(&scopes)?;
            let color = args.color.should_color();
            let output = if args.tracked {
                app.list_tracked(
                    &scopes,
                    &args.include,
                    &args.exclude,
                    args.path.as_deref(),
                    color,
                )?
            } else {
                app.list_history(HistoryQuery {
                    scopes: &scopes,
                    include: &args.include,
                    exclude: &args.exclude,
                    path: args.path.as_deref(),
                    all: args.all,
                    since: args.since.as_deref(),
                    until: args.until.as_deref(),
                    clean: args.clean_view,
                    color,
                })?
            };
            emit_output(&output, args.pager)?;
        }
        Some(Commands::Diff(args)) => {
            let scope = resolve_write_scope(
                &app,
                args.scope_flags,
                Some(args.path.to_string_lossy().as_ref()),
            )?;
            ensure_privileged_scope(scope)?;
            let mode = match args.action {
                ToggleAction::Enable => DiffMode::Unified,
                ToggleAction::Disable => DiffMode::MetadataOnly,
            };
            println!(
                "{}",
                app.set_diff_mode(scope, args.path.to_string_lossy().as_ref(), mode)?
            );
        }
        Some(Commands::Redact(args)) => {
            let scope = resolve_write_scope(
                &app,
                args.scope_flags,
                Some(args.path.to_string_lossy().as_ref()),
            )?;
            ensure_privileged_scope(scope)?;
            let mode = match args.action {
                ToggleAction::Enable => RedactionMode::Auto,
                ToggleAction::Disable => RedactionMode::Off,
            };
            println!(
                "{}",
                app.set_redaction_mode(scope, args.path.to_string_lossy().as_ref(), mode)?
            );
        }
        None => {
            let mut command = Cli::command();
            command.print_help()?;
            println!();
        }
    }

    Ok(())
}

#[derive(Parser, Debug)]
#[command(
    name = "changed",
    version,
    about = "Lightweight system tuning changelog",
    long_about = None,
    override_usage = "changed <command> [options]",
    after_help = "Examples:\n  changed init\n  changed track -U ~/.config/fish/config.fish\n  sudo changed track -S /boot/loader/entries/arch.conf\n  changed status\n  changed list -C\n  changed list -U -C\n  sudo changed list -SU -a\n\nRun `changed <command> --help` for command-specific help."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(
        about = "Initialize config, state, and default presets",
        after_help = "Behavior:\n  Create config and state directories\n  Detect host-specific presets\n  Enable default tracking presets\n  Print the initial tracking summary"
    )]
    Init(InitArgs),
    #[command(
        about = "Run the tracking daemon in the foreground",
        after_help = "Examples:\n  changed daemon -U\n  sudo changed daemon -S --once"
    )]
    Daemon(DaemonArgs),
    #[command(
        about = "Manage the changed systemd service",
        after_help = "Notes:\n  Service commands require an explicit scope.\n  `install` writes a generated unit for local/dev or non-packaged installs.\n  For packaged installs, use `systemctl enable --now changedd.service` or\n  `systemctl --user enable --now changedd.service` directly.\n\nExamples:\n  changed service install -U\n  changed service start -U\n  sudo changed service install -S\n  sudo changed service status -S"
    )]
    Service(ServiceArgs),
    #[command(
        about = "Show operational diagnostics for changed",
        after_help = "Notes:\n  With no scope flags, `changed status` defaults to user scope.\n  Use `-SU` for a merged status view across both scopes.\n\nExamples:\n  changed status\n  changed status -U\n  sudo changed status -S\n  sudo changed status -SU"
    )]
    Status(StatusArgs),
    #[command(
        about = "Manage recorded history data",
        after_help = "Examples:\n  changed history clear -U\n  sudo changed history clear -S\n  sudo changed history clear -SU"
    )]
    History(HistoryArgs),
    #[command(
        about = "Add a tracked file, category, or package target",
        override_usage = "changed track [scope] <file_path>\n       changed track [scope] category <name>\n       changed track [scope] package <manager> <package_name>",
        after_help = "Scope:\n  -S, --system          Track in system scope\n  -U, --user            Track in user scope\n\nNotes:\n  Writes must target exactly one scope.\n  Paths may infer scope automatically when obvious.\n\nExamples:\n  changed track -U ~/.config/fish/config.fish\n  changed track ~/.config/fish/config.fish -U\n  sudo changed track -S /boot/loader/entries/arch.conf\n  changed track -U category shell"
    )]
    Track(TrackArgs),
    #[command(
        about = "Remove a tracked file, category, or package target",
        override_usage = "changed untrack [scope] <file_path>\n       changed untrack [scope] category <name>\n       changed untrack [scope] package <manager> <package_name>",
        after_help = "Scope:\n  -S, --system          Untrack from system scope\n  -U, --user            Untrack from user scope\n\nNotes:\n  Writes must target exactly one scope.\n  `-SU` is invalid for write operations.\n\nExamples:\n  changed untrack -U ~/.config/fish/config.fish\n  changed untrack ~/.config/fish/config.fish -U\n  sudo changed untrack -S /boot/loader/entries/arch.conf"
    )]
    Untrack(UntrackArgs),
    #[command(
        about = "Show change history or tracked targets",
        after_help = "Notes:\n  With no scope flags, `changed list` defaults to user scope.\n\nExamples:\n  changed list\n  changed list -U\n  sudo changed list -S\n  sudo changed list -SU -a -C\n  changed list -i services\n  changed list -e packages\n  changed list -SU -C -i cpu -i gpu -e services"
    )]
    List(ListArgs),
    #[command(
        about = "Enable or disable line-diff storage for a path",
        after_help = "Examples:\n  sudo changed diff -S enable /boot/loader/entries/arch.conf\n  changed diff -U disable ~/.config/fish/config.fish"
    )]
    Diff(DiffArgs),
    #[command(
        about = "Enable or disable redaction for a path",
        after_help = "Examples:\n  changed redact -U enable ~/.config/fish/config.fish\n  sudo changed redact -S disable /etc/makepkg.conf"
    )]
    Redact(RedactArgs),
}

#[derive(Args, Debug, Clone, Copy)]
struct ScopeFlags {
    #[arg(short = 'S', long = "system", help = "Use system scope")]
    system: bool,
    #[arg(short = 'U', long = "user", help = "Use user scope")]
    user: bool,
}

impl ScopeFlags {
    fn resolve_read_scopes(self) -> Vec<Scope> {
        match (self.system, self.user) {
            (false, false) | (false, true) => vec![Scope::User],
            (true, false) => vec![Scope::System],
            (true, true) => vec![Scope::System, Scope::User],
        }
    }

    fn resolve_single(self) -> Result<Scope> {
        self.resolve_single_with_message("Error: unclear scope. Please specify -S or -U.")
    }

    fn resolve_single_with_message(self, missing_message: &str) -> Result<Scope> {
        match (self.system, self.user) {
            (true, false) => Ok(Scope::System),
            (false, true) => Ok(Scope::User),
            (true, true) => Err(anyhow!(
                "Writes must target exactly one scope. `-SU` is invalid here."
            )),
            (false, false) => Err(anyhow!(missing_message.to_owned())),
        }
    }

    fn resolve_single_or_default(self, default: Scope) -> Result<Scope> {
        match (self.system, self.user) {
            (false, false) => Ok(default),
            _ => self.resolve_single(),
        }
    }

    fn resolve_optional(self) -> Result<Option<Scope>> {
        match (self.system, self.user) {
            (false, false) => Ok(None),
            _ => self.resolve_single().map(Some),
        }
    }

    fn resolve_history_clear_scopes(self) -> Result<Vec<Scope>> {
        match (self.system, self.user) {
            (false, false) => Err(anyhow!(
                "History commands require an explicit scope. Please specify -S, -U, or -SU."
            )),
            (true, false) => Ok(vec![Scope::System]),
            (false, true) => Ok(vec![Scope::User]),
            (true, true) => Ok(vec![Scope::System, Scope::User]),
        }
    }
}

#[derive(Args, Debug)]
struct InitArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
}

#[derive(Args, Debug)]
struct TrackArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
    #[arg(value_name = "TARGET", hide = true)]
    args: Vec<String>,
}

impl TrackArgs {
    fn mode(&self) -> TrackMode {
        match self.args.as_slice() {
            [path] => TrackMode::File(path.clone()),
            [kind, name] if kind == "category" => {
                parse_category(name).map_or(TrackMode::Invalid, TrackMode::Category)
            }
            [kind, manager, package_name] if kind == "package" => TrackMode::Package {
                manager: manager.clone(),
                package_name: package_name.clone(),
            },
            _ => TrackMode::Invalid,
        }
    }
}

enum TrackMode {
    File(String),
    Category(Category),
    Package {
        manager: String,
        package_name: String,
    },
    Invalid,
}

#[derive(Args, Debug)]
struct UntrackArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
    #[arg(value_name = "TARGET", hide = true)]
    args: Vec<String>,
}

impl UntrackArgs {
    fn mode(&self) -> UntrackMode {
        match self.args.as_slice() {
            [path] => UntrackMode::File(path.clone()),
            [kind, name] if kind == "category" => {
                parse_category(name).map_or(UntrackMode::Invalid, UntrackMode::Category)
            }
            [kind, manager, package_name] if kind == "package" => UntrackMode::Package {
                manager: manager.clone(),
                package_name: package_name.clone(),
            },
            _ => UntrackMode::Invalid,
        }
    }
}

enum UntrackMode {
    File(String),
    Category(Category),
    Package {
        manager: String,
        package_name: String,
    },
    Invalid,
}

#[derive(Args, Debug)]
struct ListArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
    #[arg(
        short = 't',
        long = "tracked",
        help = "Show tracked targets instead of change events"
    )]
    tracked: bool,
    #[arg(short = 'i', long = "include", value_name = "CATEGORY", value_enum, action = clap::ArgAction::Append, help = "Include only matching categories")]
    include: Vec<Category>,
    #[arg(short = 'e', long = "exclude", value_name = "CATEGORY", value_enum, action = clap::ArgAction::Append, help = "Exclude matching categories")]
    exclude: Vec<Category>,
    #[arg(short = 'p', long = "path", value_name = "PATH", value_hint = ValueHint::AnyPath, help = "Filter by exact tracked path")]
    path: Option<PathBuf>,
    #[arg(short = 'a', long = "all", help = "Show full retained history")]
    all: bool,
    #[arg(
        short = 's',
        long = "since",
        value_name = "TIME",
        help = "Show entries since TIME (RFC3339)"
    )]
    since: Option<String>,
    #[arg(
        short = 'u',
        long = "until",
        value_name = "TIME",
        help = "Show entries until TIME (RFC3339)"
    )]
    until: Option<String>,
    #[arg(
        short = 'C',
        long = "clean-view",
        help = "Show a low-noise view of relevant changes"
    )]
    clean_view: bool,
    #[arg(long = "color", value_enum, default_value_t = ColorMode::Auto, help = "Control color output")]
    color: ColorMode,
    #[arg(
        long = "pager",
        help = "Open output in $PAGER (or less -R) instead of printing directly"
    )]
    pager: bool,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, Eq, PartialEq)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    fn should_color(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }
}

#[derive(Args, Debug)]
struct DaemonArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
    #[arg(long = "once", help = "Run one scan cycle and exit")]
    once: bool,
}

#[derive(Args, Debug)]
struct ServiceArgs {
    #[arg(value_enum)]
    action: ServiceAction,
    #[command(flatten)]
    scope_flags: ScopeFlags,
}

#[derive(Args, Debug)]
struct StatusArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
    #[arg(
        long = "pager",
        help = "Open output in $PAGER (or less -R) instead of printing directly"
    )]
    pager: bool,
}

#[derive(Args, Debug)]
struct HistoryArgs {
    #[command(subcommand)]
    command: HistoryCommand,
}

#[derive(Args, Debug)]
struct DiffArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
    #[arg(value_enum)]
    action: ToggleAction,
    #[arg(value_name = "PATH", value_hint = ValueHint::AnyPath)]
    path: PathBuf,
}

#[derive(Args, Debug)]
struct RedactArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
    #[arg(value_enum)]
    action: ToggleAction,
    #[arg(value_name = "PATH", value_hint = ValueHint::AnyPath)]
    path: PathBuf,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ServiceAction {
    Install,
    Start,
    Stop,
    Status,
}

#[derive(Subcommand, Debug)]
enum HistoryCommand {
    #[command(
        about = "Clear stored journal data for one or both scopes",
        after_help = "Examples:\n  changed history clear -U\n  sudo changed history clear -S\n  sudo changed history clear -SU"
    )]
    Clear(HistoryClearArgs),
}

#[derive(Args, Debug)]
struct HistoryClearArgs {
    #[command(flatten)]
    scope_flags: ScopeFlags,
}

impl ServiceAction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Status => "status",
        }
    }
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ToggleAction {
    Enable,
    Disable,
}

fn resolve_write_scope(app: &App, flags: ScopeFlags, path: Option<&str>) -> Result<Scope> {
    if let Some(scope) = flags.resolve_optional()? {
        Ok(scope)
    } else {
        let path = path.ok_or_else(|| anyhow!("Error: unclear scope. Please specify -S or -U."))?;
        app.infer_scope_for_path(path)?
            .ok_or_else(|| anyhow!("Error: unclear scope. Please specify -S or -U."))
    }
}

fn parse_category(raw: &str) -> Option<Category> {
    Category::ALL
        .into_iter()
        .find(|category| category.as_str() == raw)
}

fn ensure_privileged_scopes(scopes: &[Scope]) -> Result<()> {
    if scopes.contains(&Scope::System) {
        ensure_privileged_scope(Scope::System)?;
    }
    Ok(())
}

fn ensure_privileged_scope(scope: Scope) -> Result<()> {
    if scope == Scope::System && !Uid::effective().is_root() {
        return Err(anyhow!(
            "system scope requires elevated privileges. Re-run with sudo or use -U."
        ));
    }
    Ok(())
}

fn confirm_history_clear(scopes: &[Scope]) -> Result<bool> {
    let scope_label = match scopes {
        [Scope::User] => "user".to_owned(),
        [Scope::System] => "system".to_owned(),
        [Scope::System, Scope::User] | [Scope::User, Scope::System] => "user and system".to_owned(),
        _ => scopes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" and "),
    };

    let warning_line = if should_style_terminal() {
        "\x1b[1;31mWARNING: THIS COMMAND IS DESTRUCTIVE\x1b[0m".to_string()
    } else {
        String::from("WARNING: THIS COMMAND IS DESTRUCTIVE")
    };
    let prompt_line = if should_style_terminal() {
        format!("Are you sure you want to clear \x1b[1;33m{scope_label}\x1b[0m history? [y/N]: ")
    } else {
        format!("Are you sure you want to clear {scope_label} history? [y/N]: ")
    };

    print!("{warning_line}\n{prompt_line}");
    io::stdout().flush()?;

    let mut input = String::new();
    loop {
        input.clear();
        io::stdin().read_line(&mut input)?;
        match input.trim().to_ascii_lowercase().as_str() {
            "" | "n" | "no" => return Ok(false),
            "y" | "yes" => return Ok(true),
            _ => {
                if should_style_terminal() {
                    print!("\x1b[33mPlease answer y, n, yes, or no:\x1b[0m ");
                } else {
                    print!("Please answer y, n, yes, or no: ");
                }
                io::stdout().flush()?;
            }
        }
    }
}

fn should_style_terminal() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn emit_output(output: &str, pager: bool) -> Result<()> {
    if pager && io::stdout().is_terminal() {
        return emit_via_pager(output);
    }

    println!("{output}");
    Ok(())
}

fn emit_via_pager(output: &str) -> Result<()> {
    let pager = std::env::var("PAGER").unwrap_or_else(|_| String::from("less -R"));
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&pager)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to launch pager `{pager}`"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(output.as_bytes())?;
        if !output.ends_with('\n') {
            stdin.write_all(b"\n")?;
        }
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("pager `{pager}` exited with status {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn list_defaults_to_user_scope() {
        let cli = Cli::parse_from(["changed", "list"]);
        let Some(Commands::List(args)) = cli.command else {
            panic!("expected list command");
        };

        assert_eq!(args.scope_flags.resolve_read_scopes(), vec![Scope::User]);
    }

    #[test]
    fn list_allows_explicit_merged_scope() {
        let cli = Cli::parse_from(["changed", "list", "-SU"]);
        let Some(Commands::List(args)) = cli.command else {
            panic!("expected list command");
        };

        assert_eq!(
            args.scope_flags.resolve_read_scopes(),
            vec![Scope::System, Scope::User]
        );
    }

    #[test]
    fn status_defaults_to_user_scope() {
        let cli = Cli::parse_from(["changed", "status"]);
        let Some(Commands::Status(args)) = cli.command else {
            panic!("expected status command");
        };

        assert_eq!(args.scope_flags.resolve_read_scopes(), vec![Scope::User]);
    }

    #[test]
    fn track_accepts_scope_before_path() {
        let cli = Cli::parse_from(["changed", "track", "-U", "/tmp/demo"]);
        let Some(Commands::Track(args)) = cli.command else {
            panic!("expected track command");
        };

        assert!(matches!(args.mode(), TrackMode::File(path) if path == "/tmp/demo"));
        assert_eq!(args.scope_flags.resolve_single().unwrap(), Scope::User);
    }

    #[test]
    fn track_accepts_scope_after_path() {
        let cli = Cli::parse_from(["changed", "track", "/tmp/demo", "-U"]);
        let Some(Commands::Track(args)) = cli.command else {
            panic!("expected track command");
        };

        assert!(matches!(args.mode(), TrackMode::File(path) if path == "/tmp/demo"));
        assert_eq!(args.scope_flags.resolve_single().unwrap(), Scope::User);
    }

    #[test]
    fn untrack_accepts_scope_after_path() {
        let cli = Cli::parse_from(["changed", "untrack", "/tmp/demo", "-U"]);
        let Some(Commands::Untrack(args)) = cli.command else {
            panic!("expected untrack command");
        };

        assert!(matches!(args.mode(), UntrackMode::File(path) if path == "/tmp/demo"));
        assert_eq!(args.scope_flags.resolve_single().unwrap(), Scope::User);
    }

    #[test]
    fn help_works_for_valid_command_prefixes() {
        for argv in [
            vec!["changed", "list", "--help"],
            vec!["changed", "status", "--help"],
            vec!["changed", "track", "--help"],
            vec!["changed", "track", "-U", "--help"],
            vec!["changed", "track", "category", "--help"],
            vec!["changed", "untrack", "--help"],
            vec!["changed", "untrack", "-U", "--help"],
            vec!["changed", "history", "clear", "--help"],
        ] {
            let err = Cli::try_parse_from(argv).expect_err("expected clap help");
            assert_eq!(err.kind(), ErrorKind::DisplayHelp);
        }
    }
}
