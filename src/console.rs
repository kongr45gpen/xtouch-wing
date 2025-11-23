//! WING Console Interface

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use libwing::{WingConsole, WingNodeData, WingResponse};
use log::{debug, error, info, warn};
use rosc::{OscMessage, OscPacket, OscType, decoder, encoder};
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;

use crate::orchestrator::{Interface, Value};

/// WING connection
pub struct Console {
    wing: WingConsole,
    remote_addr: String,

    interface: Arc<Mutex<Option<Interface>>>,
}

impl Console {
    /// Create and connect a new Console (async).
    pub async fn new(remote_addr: &str, local_port: u16) -> Result<Self> {
        use colored::Colorize;

        let wing = WingConsole::connect(Some(remote_addr)).with_context(|| {
            format!(
                "Failed to connect to Wing console at remote address {}",
                remote_addr
            )
        })?;

        debug!("Successfully connected to Wing console at {}", remote_addr);

        let mut console = Self {
            wing,
            remote_addr: remote_addr.to_string(),
            interface: Mutex::new(None).into(),
        };

        // Initialise NAME_TO_DEF map, otherwise it will happen during a definition, which is not great.
        std::hint::black_box(WingConsole::name_to_id("/$syscfg/$cnscfg"));

        console.spawn_subscribe_task();
        console.spawn_recv_task();

        info!("OSC connected to {}", remote_addr.green());

        Ok(console)
    }

    /// Send an OSC "identify" query and wait (with timeout) for a response.
    async fn identify(interface: &Interface) -> Result<String> {
        debug!("Attempting to identify console...");

        // let interface = self.interface
        //     .lock()
        //     .await
        //     .as_ref()
        //     .ok_or_else(|| anyhow!("Interface not set up, cannot identify just now."))?
        //     // The interface lock has to be released, so that the RX part can be used.
        //     .clone();

        let result = interface
            .get_value("/$syscfg/$cnscfg", true)
            .await?;

        match result {
            Value::Str(s) => Ok(s),
            _ => bail!("Unexpected value type returned for identify query"),
        }
    }

    fn spawn_subscribe_task(&self) {
    }

    /// Spawn a background tokio task that listens for incoming OSC packets
    /// and updates the parameter cache.
    fn spawn_recv_task(&mut self) {
        let mut wing = self.wing.clone();
        let interface = self.interface.clone();

        tokio::spawn(async move {
            loop {
                let wing_read = wing.read();
                match wing_read {
                    Ok(data) => match data {
                        WingResponse::NodeData(id, data) => {
                            Console::process_node_data(interface.clone(), id, data).await;
                        }
                        WingResponse::RequestEnd => {}
                        WingResponse::NodeDef(_) => {}
                    },
                    Err(libwing::Error::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                        // Just a simple timeout, nothing to worry about
                    }
                    Err(e) => {
                        warn!("Error during OSC reception: {:?}", e);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });
    }

    /// Decode raw UDP bytes into OSC packets and update the cache.
    async fn process_node_data(
        interface: Arc<Mutex<Option<Interface>>>,
        node_id: i32,
        data: WingNodeData,
    ) {
        let node_defn = WingConsole::id_to_defs(node_id);

        if let None = node_defn {
            warn!("Unknown Node ID {} received for node data", node_id);
            return;
        }

        let node_defn = node_defn.unwrap();

        if node_defn.is_empty() || node_defn.len() > 1 {
            warn!(
                "Unexpected number of definitions ({}) for Node ID {}, must be 1. Ignoring received data.",
                node_defn.len(),
                node_id
            );
            return;
        }

        let node_addr = &node_defn[0].0;

        let value;

        // Even though the data may contain multiple values/value types, we employ a certain priority.
        if data.has_float() {
            value = Value::Float(data.get_float());
        } else if data.has_int() {
            value = Value::Int(data.get_int());
        } else if data.has_string() {
            value = Value::Str(data.get_string().to_string());
        } else {
            warn!("Node data for {} has no supported value types", node_addr);
            return;
        }

        Self::handle_value(interface, node_addr, value).await;
    }

    /// Handle a single OSC message and update the cache.
    async fn handle_value(interface: Arc<Mutex<Option<Interface>>>, node_addr: &str, data: Value) {
        use colored::Colorize;

        debug!(
            "{} OSC value: {:20} {:?}",
            "Received".green(),
            node_addr.cyan(),
            data
        );

        if let Some(iface) = interface.lock().await.as_ref() {
            iface.set_value(&node_addr, data).await;
        } else {
            warn!("No interface set to handle OSC message");
        }
    }

    /// Performs a request for an OSC value, without returning it.
    pub async fn request_value(&mut self, osc_addr: &str) -> Result<()> {
        use colored::Colorize;

        let node_id = WingConsole::name_to_id(osc_addr).with_context(|| {
            format!(
                "When requesting value, failed to get Node ID for OSC address {}",
                osc_addr
            )
        })?;

        debug!(
            "{} OSC value: {:18}",
            "Requesting".yellow(),
            osc_addr.cyan()
        );

        self.wing
            .request_node_data(node_id)
            .with_context(|| format!("Failed to request node data for ID {}", node_id))?;

        Ok(())
    }

    /// Set an OSC value
    pub async fn set_value(&mut self, osc_addr: &str, value: Value) -> Result<()> {
        use colored::Colorize;

        debug!(
            "{} OSC value: {:22} {:?}",
            "Setting".yellow(),
            osc_addr.cyan(),
            value
        );

        let node_id = WingConsole::name_to_id(osc_addr).with_context(|| {
            format!(
                "When setting value, failed to get Node ID for OSC address {}",
                osc_addr
            )
        })?;

        let result = match value {
            Value::Float(f) => self.wing.set_float(node_id, f),
            Value::Int(i) => self.wing.set_int(node_id, i),
            Value::Str(s) => self.wing.set_string(node_id, &s),
        };

        result.with_context(|| format!("Failed to set node data for ID {}", node_id))
    }

    pub async fn set_interface(&mut self, interface: Interface) {
        use colored::Colorize;

        let cloned_interface_for_later = interface.clone();

        self.interface.lock().await.replace(interface);

        tokio::spawn(async move {
                match Self::identify(&cloned_interface_for_later).await {
                    Ok(id_string) => info!("Console identified as {}", id_string.yellow().bold()),
                    Err(e) => error!("Failed to identify console: {:?}", e),
                }
        });
    }
}
