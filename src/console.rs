//! WING Console Interface

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use log::{debug, error, info, warn};
use rosc::{OscMessage, OscPacket, OscType, decoder, encoder};
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;

use crate::orchestrator::{Interface, Value};

/// WING connection and parameter cache
pub struct Console {
    socket: Arc<UdpSocket>,
    remote_addr: String,
    /// A list of currently known OSC parameters. This will be kept up to date by the
    /// subscription.
    interface: Arc<Mutex<Option<Interface>>>,
}

impl Console {
    /// Create and connect a new Console (async).
    pub async fn new(remote_addr: &str, local_port: u16) -> Result<Self> {
        use colored::Colorize;

        let local_addr = format!("0.0.0.0:{}", local_port);
        let socket = UdpSocket::bind(&local_addr)
            .await
            .with_context(|| format!("Failed to bind UDP socket to {}", local_addr))?;

        socket
            .connect(remote_addr)
            .await
            .with_context(|| format!("Failed to connect UDP socket to {}", remote_addr))?;
        let socket = Arc::new(socket);

        let console = Self {
            socket: socket.clone(),
            remote_addr: remote_addr.to_string(),
            interface: Mutex::new(None).into(),
        };

        console
            .identify()
            .await
            .with_context(|| "Failed to identify OSC device")?;

        console.spawn_subscribe_task();
        console.spawn_recv_task();

        info!(
            "OSC connected to {} (local bound to {})",
            remote_addr.green(),
            local_addr
        );

        Ok(console)
    }

    /// Send an OSC "identify" query and wait (with timeout) for a response.
    async fn identify(&self) -> Result<()> {
        let osc_msg = OscPacket::Message(OscMessage {
            addr: "/?".to_string(),
            args: vec![],
        });
        let buf = encoder::encode(&osc_msg).with_context(|| "Failed to encode OSC packet")?;
        self.socket.send(&buf).await?;

        let mut recv_buf = [0u8; 1024];

        match timeout(Duration::from_secs(2), self.socket.recv(&mut recv_buf)).await {
            Ok(Ok(size)) => {
                let (_, packet) = decoder::decode_udp(&recv_buf[..size])
                    .with_context(|| "Failed to decode OSC packet")?;

                debug!("Received OSC identification response: {:?}", packet);
            }
            Ok(Err(e)) => {
                bail!("Error receiving during OSC identification: {}", e);
            }
            Err(_) => {
                bail!("No response received during OSC identification: timeout");
            }
        }

        Ok(())
    }

    fn spawn_subscribe_task(&self) {
        let socket = self.socket.clone();

        tokio::spawn(async move {
            let subscribe_message = OscPacket::Message(OscMessage {
                addr: "/*S".to_string(),
                args: vec![],
            });

            let subscribe_message = encoder::encode(&subscribe_message)
                .with_context(|| "Failed to encode OSC subscribe packet")
                .unwrap();

            debug!("Starting OSC subscription");

            loop {
                if let Err(e) = socket.send(&subscribe_message).await {
                    warn!("Failed to send OSC subscribe packet: {}", e);
                }
                tokio::time::sleep(Duration::from_secs(8)).await;
            }
        });
    }

    /// Spawn a background tokio task that listens for incoming OSC packets
    /// and updates the parameter cache.
    fn spawn_recv_task(&self) {
        let socket = self.socket.clone();
        let interface = self.interface.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 2048];

            loop {
                match socket.recv(&mut buf).await {
                    Ok(size) => {
                        // hand raw UDP bytes to the packet processor (it will decode)
                        Console::process_packet_bytes(interface.clone(), &buf[..size]).await;
                    }
                    Err(e) => {
                        warn!("Error receiving OSC packet: {}", e);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        });
    }

    /// Decode raw UDP bytes into OSC packets and update the cache.
    async fn process_packet_bytes(interface: Arc<Mutex<Option<Interface>>>, data: &[u8]) {
        match decoder::decode_udp(data) {
            Ok((_, packet)) => match packet {
                OscPacket::Message(msg) => {
                    Console::handle_message(interface, msg).await;
                }
                OscPacket::Bundle(_) => {
                    error!("I am not equipped to handle OSC bundles, I hope they don't show up!");
                }
            },
            Err(e) => {
                warn!("Failed to decode incoming OSC packet: {}", e);
            }
        }
    }

    /// Handle a single OSC message and update the cache.
    async fn handle_message(interface: Arc<Mutex<Option<Interface>>>, msg: OscMessage) {
        debug!("Received OSC message: {:20} args={:?}", msg.addr, msg.args);

        let addr = msg.addr.clone();
        let arg = if msg.args.len() == 3 {
            msg.args.last()
        } else {
            msg.args.first()
        };
        // let mut guard = cache.write().await;

        if let Some(arg) = arg {
            let value = match arg {
                OscType::Float(f) => Value::Float(*f),
                OscType::Int(i) => Value::Int(*i),
                OscType::String(s) => Value::Str(s.clone()),
                OscType::Blob(b) => Value::Blob(b.clone()),
                _ => {
                    warn!("Unsupported OSC argument type for {}: {:?}", addr, arg);
                    return;
                }
            };

            // guard.insert(addr.clone(), value.clone());

            // interface.lock().await.inspect(async |iface: &Interface| { iface.set_value(&addr, value).await; });
            if let Some(iface) = interface.lock().await.as_ref() {
                iface.set_value(&addr, value).await;
            }
        } else {
            warn!("OSC message {} has no arguments", msg.addr);
        }
    }

    /// Performs a request for an OSC value, without returning it.
    pub async fn request_value(&self, osc_addr: &str) -> Result<()> {
        let osc_msg = OscPacket::Message(OscMessage {
            addr: osc_addr.to_string(),
            args: vec![],
        });
        let buf = encoder::encode(&osc_msg).with_context(|| "Failed to encode OSC packet")?;
        self.socket.send(&buf).await?;
        Ok(())
    }

    /// Set an OSC value
    pub async fn set_value(&self, osc_addr: &str, value: Value) -> Result<()> {
        let osc_type = match value {
            Value::Float(f) => OscType::Float(f),
            Value::Int(i) => OscType::Int(i),
            Value::Str(s) => OscType::String(s),
            Value::Blob(b) => OscType::Blob(b),
        };
        let osc_msg = OscPacket::Message(OscMessage {
            addr: osc_addr.to_string(),
            args: vec![osc_type],
        });
        let buf = encoder::encode(&osc_msg).with_context(|| "Failed to encode OSC packet")?;
        self.socket.send(&buf).await?;
        Ok(())
    }

    pub async fn set_interface(&mut self, interface: Interface) {
        self.interface.lock().await.replace(interface);
    }
}
