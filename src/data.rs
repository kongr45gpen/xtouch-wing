//! Common data types

use anyhow::{Result, bail};
use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
enum FaderType {
    Channel,
    Aux,
    Bus,
    Main,
    Matrix,
    DCA,
}

#[derive(Debug, Clone, PartialEq)]
enum PathType {
    Fader,
    Panning,
    Mute,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fader {
    osc_directory: String,
    fader_type: FaderType,
    /// Meter definition as (group byte, meter byte)
    wing_meter: Option<(u8, u8)>,
}

impl Fader {
    pub fn get_osc_path(&self, path_type: PathType) -> String {
        match path_type {
            PathType::Fader => format!("{}/fdr", self.osc_directory),
            PathType::Panning => format!("{}/pan", self.osc_directory),
            PathType::Mute => format!("{}/mute", self.osc_directory),
        }
    }

    pub fn new_from_label(label: &str) -> Result<Self> {
        // Label has format: "Channel 1"/"Matrix 4"
        let re = Regex::new(r"^(\w+)\s*(\d+)?$").unwrap();
        if let Some(caps) = re.captures(label) {
            let base = caps.get(1).unwrap().as_str().to_lowercase();
            let index = caps.get(2).map(|m| m.as_str().to_lowercase());

            let fader_type: FaderType = match base.as_str() {
                "channel" | "ch" | "chan" => FaderType::Channel,
                "aux" => FaderType::Aux,
                "bus" => FaderType::Bus,
                "main" | "lr" => FaderType::Main,
                "matrix" | "mtx" => FaderType::Matrix,
                "dca" => FaderType::DCA,
                _ => bail!("Unknown fader type: {}", base),
            };

            if let Some(index) = index {
                let osc_directory = match fader_type {
                    FaderType::Channel => format!("/ch/{}", index),
                    FaderType::Aux => format!("/aux/{}", index),
                    FaderType::Bus => format!("/bus/{}", index),
                    FaderType::Main => format!("/main/{}", index),
                    FaderType::Matrix => format!("/mtx/{}", index),
                    FaderType::DCA => format!("/dca/{}", index),
                    _ => bail!("Unknown fader type: {}", base),
                };

                let num = index
                    .parse::<u8>()
                    .map_err(|_| anyhow::anyhow!("Invalid fader index: {}", index))?;

                let wing_meter = match fader_type {
                    FaderType::Channel => Some((0xab, num)),
                    FaderType::Aux => Some((0xac, num)),
                    FaderType::Bus => Some((0xad, num)),
                    FaderType::Main => Some((0xae, num)),
                    FaderType::Matrix => Some((0xaf, num)),
                    FaderType::DCA => Some((0xa5, num)),
                    _ => None,
                };

                Ok(Self {
                    osc_directory,
                    fader_type,
                    wing_meter,
                })
            } else {
                bail!("Fader label missing index: {}", label);
            }
        } else {
            bail!("Invalid fader label format: {}", label);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bool {
    pub last_value: bool,
    pub osc_name: String,
}

/// A value that can be read from OSC into MIDI
trait ReadFloat {
    fn read(&self) -> f32;
}

/// A boolean value that can be read from OSC into MIDI
trait ReadBool {
    fn read(&self) -> bool;
}

impl ReadFloat for Fader {
    fn read(&self) -> f32 {
        // self.last_value
        0.0
    }
}

impl ReadBool for Bool {
    fn read(&self) -> bool {
        self.last_value
    }
}
