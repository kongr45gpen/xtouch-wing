//! Common data types

use anyhow::{Result, bail};
use log::debug;
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
pub enum PathType {
    Fader,
    Panning,
    Mute,
    ScribbleColour,
    ScribbleName,
    ScribbleLed,
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
            PathType::ScribbleColour => format!("{}/$col", self.osc_directory),
            PathType::ScribbleName => format!("{}/$name", self.osc_directory),
            PathType::ScribbleLed => format!("{}led", self.osc_directory),
        }
    }

    pub fn path_matches(&self, osc_path: &str) -> Option<PathType> {
        let parts: Vec<&str> = osc_path.rsplitn(2, '/').collect();

        if parts.len() != 2 {
            return None;
        }

        if parts[1] != self.osc_directory {
            return None;
        }

        match parts[0] {
            "fdr" => Some(PathType::Fader),
            "pan" => Some(PathType::Panning),
            "mute" => Some(PathType::Mute),
            "$col" => Some(PathType::ScribbleColour),
            "$name" => Some(PathType::ScribbleName),
            "led" => Some(PathType::ScribbleLed),
            _ => None,
        }
    }

    /// Gamma correction from dB to float, adjusted for WING faders
    pub fn db_to_float(db: f64) -> f64 {
        const GAMMA: f64 = 1.333333333;
        const BETA: f64 = 10.0;
        const DELTA: f64 = -144.0;

        GAMMA.powf(db / BETA - 1.0)
    }

    /// Gamma correction from float to dB, adjusted for WING faders
    pub fn float_to_db(value: f64) -> f64 {
        const GAMMA: f64 = 1.333333333;
        const BETA: f64 = 10.0;
        const DELTA: f64 = -144.0;

        let db = BETA * (value.log(GAMMA) + 1.0);

        // Optional detent
        if db.abs() <= 0.3 {
            return 0.0;
        }

        db
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
