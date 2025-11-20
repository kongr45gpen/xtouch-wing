//! The orchestrator module is responsible for synchronising values across various providers

use std::{fmt::Debug, sync::Arc};

use anyhow::Result;
use figment::providers;
use tokio::sync::RwLock;

use crate::console::{self, Value};
use log::{debug, error, info, warn};

pub trait WriteProvider {
    fn write(&self, addr: &str, value: console::Value) -> anyhow::Result<()>;
    fn set_interface(&self, interface: Interface);
}

pub struct Orchestrator {
    // TODO: Switch to tokio synchronisation structs
    console: Arc<RwLock<console::Console>>,

    providers: Vec<Arc<Box<dyn WriteProvider>>>,
}

impl Orchestrator {
    pub async fn new(console: console::Console, providers: Vec<Arc<Box<dyn WriteProvider>>>) -> Arc<Self> {
        let mut orchestra = Arc::new(Self {
            console: Arc::new(RwLock::new(console)),
            providers: providers,
        });

        {
            orchestra.console.write().await.set_interface(Interface::new(0, orchestra.clone())).await;
        }

        for (id, provider) in orchestra.providers.iter().enumerate() {
            let interface = Interface::new(id + 1, orchestra.clone());
            provider.set_interface(interface);
        }

        orchestra
    }
}

impl Debug for Orchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Orchestrator")
            .field("console", &"console::Console")
            .field("providers", &self.providers.len())
            .finish()
    }
}

#[derive(Debug)]
pub struct Interface {
    /// Console is always 0. The rest is the index in providers + 1
    id: usize,
    orchestrator: Arc<Orchestrator>,   
}

// TODO: Is this necessary and safe?
// We only access the orchestrator through safe methods
unsafe impl Send for Interface {}
unsafe impl Sync for Interface {}

impl Interface {
    pub fn new(id: usize, orchestrator: Arc<Orchestrator>) -> Self {
        Self { id, orchestrator }
    }

    pub async fn ensure_value(&self, osc_addr: &str) -> Result<()> {
        let console = self.orchestrator.console.read().await;
        console.ensure_value(osc_addr).await
    }

    pub async fn get_value(&self, osc_addr: &str) -> Result<Option<Value>> {
        let console = self.orchestrator.console.read().await;
        console.get_value(osc_addr).await
    }

    pub async fn set_value(&self, osc_addr: &str, value: console::Value) {
        if self.id != 0 {
            // Write to console which is not part of the provider list
            // TODO: Maybe it should be
            let console = self.orchestrator.console.read().await;
            if let Err(e) = console.set_value(osc_addr, value.clone()).await {
                error!("Console failed to write {}: {:?}", osc_addr, e);
            }
        }

        for (id, provider) in self.orchestrator.providers.iter().enumerate() {
            // Do not write to self!
            if id + 1 != self.id {
                if let Err(e) = provider.write(osc_addr, value.clone()) {
                    error!("Provider {} failed to write {}: {:?}", id, osc_addr, e);
                }
            }
        }
    }
}