use std::process::ExitCode;

use clap::Parser;
use proxy_cheap_mcp::cli::{self, Cli};

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: could not start async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(cli::run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            if let Some(body) = error.api_body() {
                if let Ok(text) = serde_json::to_string_pretty(body) {
                    eprintln!("{text}");
                }
            }
            ExitCode::from(error.exit_code())
        }
    }
}

fn init_tracing(verbose: bool) {
    // chromiumoxide logs every CDP message it does not recognise at warn level;
    // that is noise for a normal login and would drown the CLI output.
    let default = if verbose {
        "debug,chromiumoxide=error,chromiumoxide::browser=error"
    } else {
        "warn,chromiumoxide=error,chromiumoxide::browser=error"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}
