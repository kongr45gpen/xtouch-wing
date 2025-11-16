//! Settings/configuration structures and management
//! 
//! Settings can be provided via external YAML file or environment variables

use figment::providers::Format;
use figment::Figment;
use log::debug;
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)] 
struct FaderAssignment {
    osc: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)] 
struct ButtonAssignment {
    osc: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConsoleSettings {
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)] 
pub(crate) struct Settings {
    pub faders: [FaderAssignment; 8],
    pub master: FaderAssignment,
    pub console: ConsoleSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            faders: [
                FaderAssignment { osc: "ch.1.fdr".to_string() },
                FaderAssignment { osc: "ch.2.fdr".to_string() },
                FaderAssignment { osc: "ch.3.fdr".to_string() },
                FaderAssignment { osc: "ch.4.fdr".to_string() },
                FaderAssignment { osc: "ch.5.fdr".to_string() },
                FaderAssignment { osc: "ch.6.fdr".to_string() },
                FaderAssignment { osc: "ch.7.fdr".to_string() },
                FaderAssignment { osc: "ch.8.fdr".to_string() },
            ],
            master: FaderAssignment { osc: "dca.1.fdr".to_string() },
            console: ConsoleSettings {
                ip: "127.0.0.1".to_string(),
                port: 2223,
            },
        }
    }
}

impl Settings {
    pub fn new() -> Result<Self, figment::Error> {
        let settings: Settings = Figment::new()
            .merge(figment::providers::Serialized::defaults(Settings::default()))
            .merge(figment::providers::Yaml::file("config.yml"))
            .merge(figment::providers::Env::prefixed("WING_").split("_"))
            .extract()?;

        debug!("Loaded settings: {:?}", settings);

        Ok(settings)
    }
}