//! The orchestrator module is responsible for synchronising values across various providers

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Ok, Result, anyhow};
use figment::providers;
use libwing::Meter;
use tracing::{debug, error, info, warn};
use tokio::sync::{Notify, RwLock};
use tokio::time::timeout;

use crate::console::Console;

const OSC_TIMEOUT: Duration = Duration::from_millis(100);

/// Value types stored in the parameter cache (replaces Fader)
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    Float(f32),
    Str(String),
}

pub trait WriteProvider {
    fn write(&self, addr: &str, value: Value) -> anyhow::Result<()>;
    fn write_meter_values(&self, values: Vec<Vec<f32>>) -> anyhow::Result<()>;
    fn set_interface(&self, interface: Interface);
}

pub struct Orchestrator {
    // TODO: Switch to tokio synchronisation structs
    console: Arc<RwLock<Console>>,

    providers: Vec<Arc<Box<dyn WriteProvider>>>,

    cache: Arc<RwLock<HashMap<String, Value>>>,
    /// A tokio Notify that is signaled whenever the cache is updated
    cache_notifier: Notify,
    /// A (provider id, osc addr)-keyed map showing whether an OSC set notification for a
    /// parameter should be suppressed.
    /// TODO: Not used
    suppressed_notifications: Arc<RwLock<HashMap<(usize, String), usize>>>,
}

impl Orchestrator {
    pub async fn new(console: Console, providers: Vec<Arc<Box<dyn WriteProvider>>>) -> Arc<Self> {
        let mut orchestra = Arc::new(Self {
            console: Arc::new(RwLock::new(console)),
            providers: providers,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_notifier: Notify::new(),
            suppressed_notifications: Arc::new(RwLock::new(HashMap::new())),
        });

        {
            orchestra
                .console
                .write()
                .await
                .set_interface(Interface::new(0, orchestra.clone()))
                .await;
        }

        for (id, provider) in orchestra.providers.iter().enumerate() {
            let interface = Interface::new(id + 1, orchestra.clone());
            provider.set_interface(interface);
        }

        orchestra
    }

    pub async fn value_exists_in_cache(&self, osc_addr: &str) -> bool {
        let cache = self.cache.read().await;
        cache.contains_key(osc_addr)
    }

    /// Get a value from the OSC cache, or None if it is not cached currently.
    pub async fn get_cached_value(&self, osc_addr: &str) -> Option<Value> {
        let cache = self.cache.read().await;
        cache.get(osc_addr).cloned()
    }

    /// Request a value for future retrieval. The result is not returned. There is no
    /// guarantee that a result will be returned.
    async fn request_value_from_console(&self, osc_addr: &str) {
        let mut console = self.console.write().await;
        if let Err(e) = console.request_value(osc_addr).await {
            error!("Failed to request value {}: {:?}", osc_addr, e);
        }
    }

    /// Request a value. If it is available in the cache, it will be returned immediately.
    /// Otherwise, a request will be made and the value awaited.
    /// Note that this may never return if a value is not found. Define your own timeout
    /// when needed.
    async fn wait_for_value(&self, osc_addr: &str, force_refresh: bool) -> Value {
        if !force_refresh {
            let cache = self.cache.read().await;
            if let Some(value) = cache.get(osc_addr) {
                return value.clone();
            }
        }

        self.request_value_from_console(osc_addr).await;

        loop {
            self.cache_notifier.notified().await;

            let cache = self.cache.read().await;
            if let Some(value) = cache.get(osc_addr) {
                return value.clone();
            }
        }
    }

    /// Notify a provider for a value update
    async fn notify_provider_by_id(&self, provider_id: usize, osc_addr: &str, value: &Value) {
        if provider_id == 0 {
            // Console
            let mut console = self.console.write().await;
            if let Err(e) = console.set_value(osc_addr, value.clone()).await {
                error!("Console failed to write {}: {:?}", osc_addr, e);
            }
        } else {
            let provider = match self.providers.get(provider_id - 1) {
                Some(p) => p,
                None => {
                    error!(
                        "Tried to notify unknown provider {} for OSC update",
                        provider_id
                    );
                    return;
                }
            };

            if let Err(e) = provider.write(osc_addr, value.clone()) {
                error!(
                    "Provider {} failed to write {}: {:?}",
                    provider_id - 1,
                    osc_addr,
                    e
                );
            }
        }
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

#[derive(Debug, Clone)]
pub struct Interface {
    /// Console is always 0. The rest is the index in providers + 1
    id: usize,
    orchestrator: Arc<Orchestrator>,
}

// TODO: Is this necessary and safe?
// We only access the orchestrator through safe methods
unsafe impl Send for Orchestrator {}
unsafe impl Sync for Orchestrator {}

impl Interface {
    pub fn new(id: usize, orchestrator: Arc<Orchestrator>) -> Self {
        Self { id, orchestrator }
    }

    /// Ensure that the value is available, requesting it if necessary.
    /// This may generate a notification that will be sent to the caller.
    pub async fn ensure_value(&self, osc_addr: &str, force_refresh: bool) {
        if !force_refresh && self.orchestrator.value_exists_in_cache(osc_addr).await {
            return;
        }

        self.orchestrator.request_value_from_console(osc_addr).await;
    }

    /// Get an OSC value, requesting it from the console if necessary.
    /// This may generate a notification that will be sent to the caller.
    /// Results to an error in case of a timeout.
    pub async fn get_value(&self, osc_addr: &str, force_refresh: bool) -> Result<Value> {
        let future = self.orchestrator.wait_for_value(osc_addr, force_refresh);

        timeout(OSC_TIMEOUT, future)
            .await
            .with_context(|| format!("Timed out waiting for value {}", osc_addr))
    }

    /// Request a value notification that contains a value.
    /// A notification is not guaranteed in case of error.
    pub async fn request_value_notification(&self, osc_addr: &str, force_refresh: bool) {
        if force_refresh || !self.orchestrator.value_exists_in_cache(osc_addr).await {
            // Requesting the value from the console will generate a notification
            self.orchestrator.request_value_from_console(osc_addr).await;
        } else {
            // If the value is in the cache, send an explicit notification
            let value = self.orchestrator.get_cached_value(osc_addr).await.unwrap();
            self.orchestrator
                .notify_provider_by_id(self.id, osc_addr, &value)
                .await;
        }
    }

    /// Request a value notification that contains an OSC value.
    /// This function will wait a bit to ensure the value is available, returning an error otherwise.
    pub async fn request_value_notification_checked(
        &self,
        osc_addr: &str,
        force_refresh: bool,
    ) -> Result<()> {
        if force_refresh || !self.orchestrator.value_exists_in_cache(osc_addr).await {
            // Requesting the value from the console will generate a notification
            let future = self.orchestrator.wait_for_value(osc_addr, force_refresh);

            timeout(OSC_TIMEOUT, future)
                .await
                .with_context(|| format!("Timed out waiting for value {}", osc_addr))?;
            Ok(())
        } else {
            // If the value is in the cache, send an explicit notification
            let value = self.orchestrator.get_cached_value(osc_addr).await.unwrap();
            self.orchestrator
                .notify_provider_by_id(self.id, osc_addr, &value)
                .await;
            Ok(())
        }
    }

    /// Set an OSC value, notifying all other providers/interfaces except self.
    /// 
    /// For example, a console can set_value, which will notify everyone else.
    pub async fn set_value(&self, osc_addr: &str, value: Value) {
        // Update cache
        self.orchestrator
            .cache
            .write()
            .await
            .insert(osc_addr.to_string(), value.clone());
        self.orchestrator.cache_notifier.notify_waiters();

        if self.id != 0 {
            // Write to console which is not part of the provider list
            let mut console = self.orchestrator.console.write().await;
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

    /// Subscribe to specific meter updates from the console.
    /// 
    /// NOTE: This will override any previous subscriptions.
    /// TODO: Make it not override any previous subscriptions.
    pub async fn subscribe_to_meters(&self, meters: Vec<Meter>) -> Result<()> {
        let mut console = self.orchestrator.console.write().await;
        debug!(interface_id = self.id, meter_count = meters.len(), "Interface subscribed to meters");
        console.set_meters(meters).await
    }

    /// Broadcast meter values.
    /// 
    /// These values are not cached, but instead are sent immediatelly to subscribers.
    /// 
    /// TODO: Use slice instead of vector
    pub(crate) async fn set_meters(&self, values: Vec<Vec<f32>>) {
        for provider in self.orchestrator.providers.iter() {
            if let Err(e) = provider.write_meter_values(values.clone()) {
                error!("Provider failed to write meter values: {:?}", e);
            }
        }
    }
}
