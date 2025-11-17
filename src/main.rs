use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;
use env_logger::Env;
use log::{debug, error, info, warn};

mod console;
mod data;
mod settings;

/// XTouch Wing - Command line options
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Activate debug mode
    #[arg(short, long)]
    debug: bool,

    /// Local UDP port to bind (default: 9001)
    #[arg(long, default_value_t = 9001)]
    local_port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set log level based on debug flag
    let log_level = if cli.debug { "debug" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(log_level)).init();

    let config =
        settings::Settings::new().with_context(|| "Failed to load configuration settings")?;

    if cli.debug {
        debug!("{}", "Debug mode is enabled".yellow());
    }
    info!("{}", "XTouch Wing started".green());

    // OSC connection logic
    let remote_addr = format!("{}:{}", config.console.ip, config.console.port);
    let console = console::Console::new(&remote_addr, cli.local_port)
        .await
        .with_context(|| "Failed to create OSC console connection")?;

    // TODO: Use a proper runtime, wait until all tasks are complete
    tokio::time::sleep(tokio::time::Duration::from_secs(6000)).await;

    Ok(())
}
