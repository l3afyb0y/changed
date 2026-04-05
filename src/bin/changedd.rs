use changed::app::{App, DaemonOptions};
use changed::scope::Scope;
use nix::unistd::Uid;
use std::env;

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let (scope, options) = parse_args(env::args().skip(1))?;
    ensure_privileged_scope(scope)?;
    let app = App::new()?;
    println!("{}", app.run_daemon(scope, options)?);
    Ok(())
}

fn parse_args<I>(args: I) -> anyhow::Result<(Scope, DaemonOptions)>
where
    I: IntoIterator<Item = String>,
{
    let mut scope = Scope::User;
    let mut once = false;
    let args = args.into_iter();

    for arg in args {
        match arg.as_str() {
            "--system" => scope = Scope::System,
            "--user" => scope = Scope::User,
            "--once" => once = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("changedd {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => {
                return Err(anyhow::anyhow!("unrecognized argument: {other}"));
            }
        }
    }

    Ok((scope, DaemonOptions { once }))
}

fn print_help() {
    println!(
        "changedd - dedicated daemon for changed\n\n\
Usage:\n  changedd [options]\n\n\
Options:\n  --once                     Run one scan cycle and exit\n  --system                   Run in system scope\n  --user                     Run in user scope\n  -h, --help                 Show this help text\n  -V, --version              Show version\n\n\
Examples:\n  changedd --user\n  changedd --system --once"
    );
}

fn ensure_privileged_scope(scope: Scope) -> anyhow::Result<()> {
    if scope == Scope::System && !Uid::effective().is_root() {
        return Err(anyhow::anyhow!(
            "system scope requires elevated privileges. Re-run with sudo or use --user."
        ));
    }
    Ok(())
}
