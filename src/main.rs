use anyhow::Result;
use changed::app::{App, DaemonOptions};
use changed::category::Category;
use changed::config::{DiffMode, RedactionMode};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueHint};
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let app = App::new()?;

    match cli.command {
        Some(Commands::Init) => println!("{}", app.init()?),
        Some(Commands::Daemon(args)) => {
            let options = DaemonOptions {
                once: args.once,
                interval: Duration::from_secs(args.interval_seconds),
            };
            println!("{}", app.run_daemon(options)?);
        }
        Some(Commands::Service { action }) => {
            println!("{}", app.service_message(action.as_str()));
        }
        Some(Commands::Track(args)) => match args.mode() {
            TrackMode::File(path) => println!("{}", app.track_file(&path)?),
            TrackMode::Category(category) => println!("{}", app.track_category(category)?),
            TrackMode::Package {
                manager,
                package_name,
            } => println!("{}", app.track_package(&manager, &package_name)?),
            TrackMode::Invalid => {
                eprintln!("Invalid track usage. Run `changed track --help` for details.");
                std::process::exit(2);
            }
        },
        Some(Commands::Untrack(args)) => match args.mode() {
            UntrackMode::File(path) => println!("{}", app.untrack_file(&path)?),
            UntrackMode::Category(category) => println!("{}", app.untrack_category(category)?),
            UntrackMode::Package {
                manager,
                package_name,
            } => println!("{}", app.untrack_package(&manager, &package_name)?),
            UntrackMode::Invalid => {
                eprintln!("Invalid untrack usage. Run `changed untrack --help` for details.");
                std::process::exit(2);
            }
        },
        Some(Commands::List(args)) => {
            let output = if args.tracked {
                app.list_tracked(args.category, args.path.as_deref())?
            } else {
                app.list_history(
                    args.category,
                    args.path.as_deref(),
                    args.all,
                    args.since.as_deref(),
                    args.until.as_deref(),
                    args.clean_view,
                )?
            };
            println!("{output}");
        }
        Some(Commands::Diff { action, path }) => {
            let mode = match action {
                ToggleAction::Enable => DiffMode::Unified,
                ToggleAction::Disable => DiffMode::MetadataOnly,
            };
            println!("{}", app.set_diff_mode(path.to_string_lossy().as_ref(), mode)?);
        }
        Some(Commands::Redact { action, path }) => {
            let mode = match action {
                ToggleAction::Enable => RedactionMode::Auto,
                ToggleAction::Disable => RedactionMode::Off,
            };
            println!(
                "{}",
                app.set_redaction_mode(path.to_string_lossy().as_ref(), mode)?
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
    after_help = "Examples:\n  changed init\n  changed track /etc/makepkg.conf\n  changed track category shell\n  changed list -c cpu\n  changed list -t\n  changed service status\n\nRun `changed <command> --help` for command-specific help."
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
    Init,
    #[command(
        about = "Run the tracking daemon in the foreground",
        after_help = "Examples:\n  changed daemon\n  changed daemon --once\n  changed daemon --interval-seconds 5"
    )]
    Daemon(DaemonArgs),
    #[command(
        about = "Manage the changed systemd service",
        after_help = "Examples:\n  changed service install\n  changed service start\n  changed service status"
    )]
    Service {
        #[arg(value_enum)]
        action: ServiceAction,
    },
    #[command(
        about = "Add a tracked file, category, or package target",
        override_usage = "changed track <file_path>\n       changed track category <name>\n       changed track package <manager> <package_name>",
        after_help = "Arguments:\n  <file_path>           Track one exact file path\n  <name>                Category name such as cpu, gpu, or services\n  <manager>             Package manager name, starting with pacman\n  <package_name>        Package name to track\n\nExamples:\n  changed track /etc/makepkg.conf\n  changed track category shell\n  changed track package pacman linux-zen"
    )]
    Track(TrackArgs),
    #[command(
        about = "Remove a tracked file, category, or package target",
        override_usage = "changed untrack <file_path>\n       changed untrack category <name>\n       changed untrack package <manager> <package_name>",
        after_help = "Arguments:\n  <file_path>           Remove one exact file path from tracking\n  <name>                Category name such as cpu, gpu, or services\n  <manager>             Package manager name, starting with pacman\n  <package_name>        Package name to stop tracking\n\nExamples:\n  changed untrack /etc/makepkg.conf\n  changed untrack category cpu\n  changed untrack package pacman linux-zen"
    )]
    Untrack(UntrackArgs),
    #[command(
        about = "Show change history or tracked targets",
        after_help = "Examples:\n  changed list\n  changed list -a\n  changed list -t\n  changed list -c services\n  changed list -C -c services\n  changed list -p /etc/makepkg.conf"
    )]
    List(ListArgs),
    #[command(
        about = "Enable or disable line-diff storage for a path",
        after_help = "Examples:\n  changed diff enable /etc/makepkg.conf\n  changed diff disable ~/.config/fish/config.fish"
    )]
    Diff {
        #[arg(value_enum)]
        action: ToggleAction,
        #[arg(value_name = "PATH", value_hint = ValueHint::AnyPath)]
        path: PathBuf,
    },
    #[command(
        about = "Enable or disable redaction for a path",
        after_help = "Examples:\n  changed redact enable ~/.config/fish/config.fish\n  changed redact disable /etc/makepkg.conf"
    )]
    Redact {
        #[arg(value_enum)]
        action: ToggleAction,
        #[arg(value_name = "PATH", value_hint = ValueHint::AnyPath)]
        path: PathBuf,
    },
}

#[derive(Args, Debug)]
struct TrackArgs {
    #[arg(value_name = "TARGET", trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    args: Vec<String>,
}

impl TrackArgs {
    fn mode(&self) -> TrackMode {
        match self.args.as_slice() {
            [] => TrackMode::Invalid,
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
    Package { manager: String, package_name: String },
    Invalid,
}

#[derive(Args, Debug)]
struct UntrackArgs {
    #[arg(value_name = "TARGET", trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    args: Vec<String>,
}

impl UntrackArgs {
    fn mode(&self) -> UntrackMode {
        match self.args.as_slice() {
            [] => UntrackMode::Invalid,
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
    Package { manager: String, package_name: String },
    Invalid,
}

#[derive(Args, Debug)]
struct ListArgs {
    #[arg(short = 't', long = "tracked", help = "Show tracked targets instead of change events")]
    tracked: bool,
    #[arg(short = 'c', long = "category", value_name = "NAME", value_enum, help = "Filter by category")]
    category: Option<Category>,
    #[arg(short = 'p', long = "path", value_name = "PATH", value_hint = ValueHint::AnyPath, help = "Filter by exact tracked path")]
    path: Option<PathBuf>,
    #[arg(short = 'a', long = "all", help = "Show full retained history")]
    all: bool,
    #[arg(short = 's', long = "since", value_name = "TIME", help = "Show entries since TIME (RFC3339)")]
    since: Option<String>,
    #[arg(short = 'u', long = "until", value_name = "TIME", help = "Show entries until TIME (RFC3339)")]
    until: Option<String>,
    #[arg(short = 'C', long = "clean-view", help = "Show a low-noise view of relevant changes")]
    clean_view: bool,
}

#[derive(Args, Debug)]
struct DaemonArgs {
    #[arg(long = "once", help = "Run one scan cycle and exit")]
    once: bool,
    #[arg(
        long = "interval-seconds",
        default_value_t = 2,
        help = "Polling interval in seconds for continuous mode"
    )]
    interval_seconds: u64,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ServiceAction {
    Install,
    Start,
    Stop,
    Status,
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

fn parse_category(raw: &str) -> Option<Category> {
    Category::ALL
        .into_iter()
        .find(|category| category.as_str() == raw)
}
