use anyhow::{Context, Result};
use rosc::{OscPacket, OscMessage, encoder};
use clap::Parser;
use colored::Colorize;
use env_logger::Env;
use log::{info, error, warn, debug};

/// XTouch Wing - Command line options
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Activate debug mode
    #[arg(short, long)]
    debug: bool,

    /// OSC server host
    #[arg(long)]
    host: String,

    /// OSC server port
    #[arg(long)]
    port: u16,

    /// Local UDP port to bind (default: 9000)
    #[arg(long, default_value_t = 9001)]
    local_port: u16,
}


fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set log level based on debug flag
    let log_level = if cli.debug { "debug" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(log_level)).init();

    if cli.debug {
        debug!("{}", "Debug mode is enabled".yellow());
    }
    info!("{}", "XTouch Wing started".green());

    // OSC connection logic
    let addr = format!("{}:{}", cli.host, cli.port);
    info!("{}", format!("Connecting to OSC device at {}", addr).cyan());

    let local_addr = format!("0.0.0.0:{}", cli.local_port);
    let socket = std::net::UdpSocket::bind(&local_addr)
        .with_context(|| format!("Failed to bind UDP socket to {}", local_addr))?;
    socket.set_read_timeout(Some(std::time::Duration::from_secs(2))).ok();
    socket.connect(&addr)
        .with_context(|| format!("Failed to connect to {}", addr))?;

    // Send a dummy OSC packet to check if device is alive
    let osc_msg = OscPacket::Message(OscMessage{
        addr: "/?".to_string(),
        args: vec![],
    });
    let buf = encoder::encode(&osc_msg)
        .with_context(|| "Failed to encode OSC packet")?;
    socket.send(&buf)
        .with_context(|| "Failed to send OSC ping packet")?;

    let mut recv_buf = [0u8; 1024];
    match socket.recv(&mut recv_buf) {
        Ok(_n) => {
            info!("{} {}", "OSC device responded: ".green(), String::from_utf8_lossy(&recv_buf));
        }
        Err(e) => {
            warn!("{}", format!("No response from OSC device: {}", e).yellow());
        }
    }

    Ok(())
}
