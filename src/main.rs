#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]
#![allow(unused_imports)]
#![allow(unused_mut)]

use anyhow::{Context, Result};
use clap::Parser;
use env_logger::Env;
use tracing::{debug, error, info, level_filters::LevelFilter, warn};
use tracing_subscriber::EnvFilter;

mod console;
mod data;
mod midi;
mod mqtt;
mod orchestrator;
mod settings;
mod utils;

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

    /// Enable vegas mode without faders (for testing)
    #[arg(long, default_value_t = false)]
    vegas_silent: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set log level based on debug flag
    let log_level = if cli.debug { LevelFilter::DEBUG } else { LevelFilter::INFO };
    // env_logger::Builder::from_env(Env::default().default_filter_or(log_level))
    //     .format_timestamp_micros()
    //     .init();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(log_level.into()))
        .with_target(true)
        .init();

    let config =
        settings::Settings::new().with_context(|| "Failed to load configuration settings")?;

    if cli.debug {
        debug!("Debug mode is enabled");
    }
    info!("XTouch Wing started");

    // OSC connection logic
    let remote_addr = format!("{}:{}", config.console.ip, config.console.port);
    let console = console::Console::new(&config.console.ip, cli.local_port)
        .await
        .with_context(|| "Failed to create OSC console connection")?;

    let mut midi = midi::Controller::new(&config.midi, &config.midi_definition)
        .with_context(|| "Failed to create MIDI controller")?;
    midi.lock().await.clean_buttons().await;

    // let mut mqtt = mqtt::Mqtt::new(&config.mqtt.host, config.mqtt.port)
    //     .await
    //     .with_context(|| "Failed to create MQTT client")?;

    if cli.vegas {
        warn!("{}", "Test run, Vegas mode");
        midi.lock().await.vegas_mode(true).await?;
    } else if cli.vegas_silent {
        warn!("{}", "Test run, Vegas mode silent");
        midi.lock().await.vegas_mode(false).await?;
    }

    let mut midi_arc = std::sync::Arc::new(Box::new(midi) as Box<dyn orchestrator::WriteProvider>);

    let mut orchestrator = orchestrator::Orchestrator::new(console, vec![midi_arc]).await;

    std::future::pending::<()>().await;

    unreachable!()
}
