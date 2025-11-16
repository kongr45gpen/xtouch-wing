//! WING Console Interface

use anyhow::{Context, Result};
use log::{debug, info, warn};
use rosc::{decoder, OscMessage, OscPacket, encoder};
use std::collections::{HashMap};
use std::net::{UdpSocket};
use std::time::Duration;

use crate::console;
use crate::data::Fader;

/// OSC connection and parameter cache
pub struct Console {
    socket: UdpSocket,
    remote_addr: String,
    pub parameter_cache: HashMap<String, Fader>,
}

impl Console {
    pub fn new(remote_addr: &str, local_port: u16) -> Result<Self> {
        use colored::Colorize;

        let local_addr = format!("0.0.0.0:{}", local_port);
        let socket = UdpSocket::bind(&local_addr)
            .with_context(|| format!("Failed to bind UDP socket to {}", local_addr))?;
        socket.set_read_timeout(Some(Duration::from_secs(2))).ok();

        socket.connect(remote_addr)
            .with_context(|| format!("Failed to connect UDP socket to {}", remote_addr))?;

        let console = Self {
            socket,
            remote_addr: remote_addr.to_string(),
            parameter_cache: HashMap::new(),
        };

        console.identify().with_context(|| "Failed to identify OSC device")?;

        info!("OSC connected to {} (local bound to {})", remote_addr.green(), local_addr);

        Ok(console)
    }

    fn identify(&self) -> Result<()> {
        let osc_msg = OscPacket::Message(OscMessage{
            addr: "/?".to_string(),
            args: vec![],
        });
        let buf = encoder::encode(&osc_msg)
            .with_context(|| "Failed to encode OSC packet")?;
        self.socket.send(&buf)?;

        let mut recv_buf = [0u8; 1024];
        match self.socket.recv(&mut recv_buf) {
            Ok(size) => {
                let packet = decoder::decode_udp(&recv_buf[..size])
                    .with_context(|| "Failed to decode OSC packet")?;
                match packet {
                    (_, msg) => {
                        debug!("Received OSC identification response: {:?}", msg);
                    },
                    _ => {
                        warn!("Unexpected OSC packet type received during identification");
                    }
                }
            },
            Err(e) => {
                warn!("No response received during OSC identification: {}", e);
            }
        }

        Ok(())
    }

    pub fn get_f32(&self, address: &str) -> Option<f32> {
        self.parameter_cache.get(address).map(|fader| fader.last_value)
    }
}
