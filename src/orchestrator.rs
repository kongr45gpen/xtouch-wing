//! The orchestrator module is responsible for synchronising values across various providers

use std::{collections::HashMap, fmt::Debug, sync::Arc};

use anyhow::{Ok, Result};
use figment::providers;
use tokio::sync::RwLock;

use log::{debug, error, info, warn};

use crate::console::Console;

/// Value types stored in the parameter cache (replaces Fader)
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    Float(f32),
    Str(String),
    Blob(Vec<u8>),
}

pub trait WriteProvider {
    fn write(&self, addr: &str, value: Value) -> anyhow::Result<()>;
    fn set_interface(&self, interface: Interface);
}

pub struct Orchestrator {
    // TODO: Switch to tokio synchronisation structs
    console: Arc<RwLock<Console>>,

    providers: Vec<Arc<Box<dyn WriteProvider>>>,

    pub cache: Arc<RwLock<HashMap<String, Value>>>,
}

impl Orchestrator {
    pub async fn new(console: Console, providers: Vec<Arc<Box<dyn WriteProvider>>>) -> Arc<Self> {
        let mut orchestra = Arc::new(Self {
            console: Arc::new(RwLock::new(console)),
            providers: providers,
            cache: Arc::new(RwLock::new(HashMap::new())),
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

    /// Get a value from the OSC cache, or None if it is not cached currently.
    pub fn get_cached_value(&self, osc_addr: &str) -> Option<Value> {
        let cache = self.cache.blocking_read();
        cache.get(osc_addr).cloned()
    }

    /// Request a value for future retrieval. The result is not returned. There is no
    /// guarantee that a result will be returned.
    pub async fn request_value(&self, osc_addr: &str) {
        let console = self.console.read().await;
        if let Err(e) = console.request_value(osc_addr).await {
            error!("Failed to request value {}: {:?}", osc_addr, e);
        }
    }

    /// Request a value. If it is available in the cache, it will be returned immediately.
    /// Otherwise, a request will be made and the value awaited. 
    pub async fn get_value(&self, osc_addr: &str) -> Option<Value> {
        {
            let cache = self.cache.read().await;
            if let Some(value) = cache.get(osc_addr) {
                return Some(value.clone());
            }
        }

        self.request_value(osc_addr).await;

        // Wait for it to appear in the cache
        for _ in 0..10 {
            {
                let cache = self.cache.read().await;
                if let Some(value) = cache.get(osc_addr) {
                    return Some(value.clone());
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        None
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

unsafe impl Send for Orchestrator {}
unsafe impl Sync for Orchestrator {}

impl Interface {
    pub fn new(id: usize, orchestrator: Arc<Orchestrator>) -> Self {
        Self { id, orchestrator }
    }

    pub async fn ensure_value(&self, osc_addr: &str) -> Result<()> {
        //TODO: Type + check if cache + remove if needed
        //TODO: What should the calling conventions be to minimise comms? Add a 'force' parameter?
        self.orchestrator.request_value(osc_addr).await;
        Ok(())
    }

    pub async fn get_value(&self, osc_addr: &str) -> Result<Option<Value>> {
        let value = self.orchestrator.get_value(osc_addr).await;
        Ok(value)
    }

    pub async fn set_value(&self, osc_addr: &str, value: Value) {
        // Update cache
        self.orchestrator.cache.write().await.insert(osc_addr.to_string(), value.clone());

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