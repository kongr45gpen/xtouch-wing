#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]
#![allow(unused_imports)]
#![allow(unused_mut)]

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;
use env_logger::Env;
use log::{debug, error, info, warn};

mod midi;
mod console;
mod data;
mod settings;
mod mqtt;

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

    /// Enable vegas mode (for testing)
    #[arg(long, default_value_t = false)]
    vegas: bool,
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

    let mut midi = midi::Controller::new(&config.midi.input, &config.midi.output)
        .with_context(|| "Failed to create MIDI controller")?;

    // let mut mqtt = mqtt::Mqtt::new(&config.mqtt.host, config.mqtt.port)
    //     .await
    //     .with_context(|| "Failed to create MQTT client")?;

    // TODO: Use a proper runtime, wait until all tasks are complete
    if cli.vegas {
        warn!("{}", "Test run, Vegas mode".yellow());
        midi.vegas_mode(true).await?;
    }
    tokio::time::sleep(tokio::time::Duration::from_secs(6000)).await;

    Ok(())
}
