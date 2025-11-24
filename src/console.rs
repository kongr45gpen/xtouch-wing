//! WING Console Interface

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use libwing::{WingConsole, WingNodeData, WingResponse};
use tracing::{Instrument, Level, debug, error, event, info, instrument, span, trace, warn};
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

    meter_task_spawned: bool,
    meters: Arc<Mutex<Vec<libwing::Meter>>>,
}

impl Console {
    /// Create and connect a new Console.
    #[instrument(name = "wing_connect", level = "info", skip_all)]
    pub async fn new(remote_addr: &str, local_port: u16) -> Result<Self> {
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
            meter_task_spawned: false,
            meters: Arc::new(Mutex::new(vec![])),
        };

        // Initialise NAME_TO_DEF map, otherwise it will happen during a request, which is not great.
        debug!("Initialising NAME_TO_DEF map...");
        std::hint::black_box(WingConsole::name_to_id("/$syscfg/$cnscfg"));
        debug!("Initialised  NAME_TO_DEF map.");

        console.spawn_recv_task();

        event!(Level::INFO, addr = remote_addr, "Console connected");

        Ok(console)
    }

    /// Send an OSC "identify" query and wait (with timeout) for a response.
    async fn identify(interface: &Interface) -> Result<String> {
        debug!("Attempting to identify console...");

        let result = interface
            .get_value("/$syscfg/$cnscfg", true)
            .await?;

        match result {
            Value::Str(s) => Ok(s),
            _ => bail!("Unexpected value type returned for identify query"),
        }
    }

    /// Spawn a background tokio task that periodically reads meter values.
    /// 
    /// ## Panics
    /// This will panic if no meters have been requested, as the internal UDP socket
    /// might not have been set up.
    fn spawn_meter_task(&self) {
        let mut wing = self.wing.clone();
        let interface = self.interface.clone();
        let meters = self.meters.clone();

        let span = span!(Level::INFO, "wing_meter_task");

        span.in_scope(|| {
            info!("Subscribing to meter updates...");
        });

        tokio::spawn(async move {
            loop {
                let meter = match wing.read_meters() {
                    Ok(m) => m,
                    Err(libwing::Error::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                        // Just a simple timeout, nothing to worry about
                        continue;
                    },
                    Err(e) => {
                        warn!("Error during meter reception: {:?}", e);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                };

                trace!(?meter, "Received meter data");

                let processed = Self::process_meter_data(meters.clone(), meter.1).await;

                trace!(?processed, "Processed meter data");

                match processed {
                    Ok(v) => {
                        let interface = interface.lock().await;
                        if let Some(iface) = interface.as_ref() {
                            iface.set_meters(v).await;
                        } else {
                            error!("No interface set to handle meter data");
                        }
                    }
                    Err(e) => {
                        warn!("Error processing meter data: {:?}", e);
                    }
                }
            }
        }.instrument(span));
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
                            let span = span!(Level::DEBUG, "osc_in", node_id = id);
                            let _enter = span.enter();

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

    /// Decode raw meter data into an array of meter values
    async fn process_meter_data(meters: Arc<Mutex<Vec<libwing::Meter>>>, data: Vec<i16>) -> Result<Vec<Vec<f32>>> {
        let meters = meters.lock().await;

        let mut data_index = 0;

        let result = meters.iter().map(|meter| {
            let count = wing_get_meter_count(meter);

            if data_index + count > data.len() {
                bail!("Not enough data received for meters. Expected {} values, got {}", count + data_index, data.len());
            }

            let values = &data[data_index..data_index + count];

            data_index += count;

           Ok( values.iter().map(|v| *v as f32 / 32768.0 + 1.0).collect::<Vec<f32>>())
        }).collect::<Result<Vec<Vec<f32>>>>();

        result
    }

    /// Decode raw data into OSC packets and update the cache.
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
        debug!(
            node_addr,
            ?data,
            "Received OSC value"
        );

        if let Some(iface) = interface.lock().await.as_ref() {
            iface.set_value(&node_addr, data).await;
        } else {
            warn!("No interface set to handle OSC message");
        }
    }

    /// Performs a request for an OSC value, without returning it.
    pub async fn request_value(&mut self, osc_addr: &str) -> Result<()> {
        let node_id = WingConsole::name_to_id(osc_addr).with_context(|| {
            format!(
                "When requesting value, failed to get Node ID for OSC address {}",
                osc_addr
            )
        })?;

        debug!(
            osc_addr,
            "Requesting OSC value"
        );

        trace!(node_id, "Requesting OSC value Node ID");

        self.wing
            .request_node_data(node_id)
            .with_context(|| format!("Failed to request node data for ID {}", node_id))?;

        Ok(())
    }

    /// Set an OSC value
    pub async fn set_value(&mut self, osc_addr: &str, value: Value) -> Result<()> {
        debug!(osc_addr, ?value, "Setting OSC value");

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
        let cloned_interface_for_later = interface.clone();

        self.interface.lock().await.replace(interface);

        tokio::spawn(async move {
                match Self::identify(&cloned_interface_for_later).await {
                    Ok(id_string) => info!(id_string, "Console identified as"),
                    Err(e) => error!("Failed to identify console: {:?}", e),
                }
        });
    }

    pub async fn set_meters(&mut self, meters: Vec<libwing::Meter>) -> Result<()> {
        {
            let mut guard = self.meters.lock().await;
            *guard = meters;
            self.wing.request_meter(&*guard).with_context(|| "Failed to request meters")?;
        }

        if self.meter_task_spawned == false {
            self.spawn_meter_task();
            self.meter_task_spawned = true;
        }

        Ok(())
    }
}

fn wing_get_meter_count(meter: &libwing::Meter) -> usize {
    use libwing::Meter;

    match meter {
        Meter::Channel(_) | Meter::Aux(_) | Meter::Bus(_) | Meter::Main(_) | Meter::Matrix(_) => 8,
        Meter::Dca(_) => 4,
        Meter::Fx(_) => 10,
        Meter::Source(_) | Meter::Output(_) => 1,
        Meter::Monitor => 6,
        Meter::Rta => 120,
        Meter::Channel2(_) | Meter::Aux2(_) | Meter::Bus2(_) | Meter::Main2(_) | Meter::Matrix2(_) => 11,
    }
}