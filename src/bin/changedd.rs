use changed::app::{App, DaemonOptions};
use std::env;
use std::time::Duration;

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let options = parse_args(env::args().skip(1))?;
    let app = App::new()?;
    println!("{}", app.run_daemon(options)?);
    Ok(())
}

fn parse_args<I>(args: I) -> anyhow::Result<DaemonOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut once = false;
    let mut interval_seconds = 2u64;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--once" => once = true,
            "--interval-seconds" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing value for --interval-seconds"))?;
                interval_seconds = value.parse::<u64>().map_err(|_| {
                    anyhow::anyhow!("invalid value for --interval-seconds: {value}")
                })?;
            }
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

    Ok(DaemonOptions {
        once,
        interval: Duration::from_secs(interval_seconds),
    })
}

fn print_help() {
    println!(
        "changedd - dedicated daemon for changed\n\n\
Usage:\n  changedd [options]\n\n\
Options:\n  --once                     Run one scan cycle and exit\n  --interval-seconds SECONDS Polling interval in seconds for fallback waiting\n  -h, --help                 Show this help text\n  -V, --version              Show version\n\n\
Examples:\n  changedd\n  changedd --once\n  changedd --interval-seconds 5"
    );
}
