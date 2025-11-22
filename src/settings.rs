//! Settings/configuration structures and management
//!
//! Settings can be provided via external YAML file or environment variables

use std::collections::HashMap;

use figment::Figment;
use figment::providers::Format;
use log::debug;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

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
pub(crate) struct FaderBank {
    pub name: Option<String>,
    pub faders: Vec<String>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerAssignments {
    pub banks: Vec<FaderBank>,
    pub fader_buttons: Vec<String>,

    pub fixed_faders: HashMap<u32, String>,
    #[serde_as(as = "Vec<(_, _)>")]
    pub fixed_buttons: HashMap<u32, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControllerSettings {
    pub input: String,
    pub output: String,

    pub assignments: ControllerAssignments,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MidiButton {
    pub channel: u8,
    pub key: u8,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MidiFader {
    pub channel: u8,
    pub buttons: Vec<MidiButton>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MidiDefinition {
    pub faders: Vec<MidiFader>,
    pub buttons: Vec<MidiButton>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MqttSettings {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub faders: [FaderAssignment; 8],
    pub master: FaderAssignment,
    pub console: ConsoleSettings,
    pub midi: ControllerSettings,
    pub midi_definition: MidiDefinition,
    pub mqtt: MqttSettings,
}

impl ControllerAssignments {
    /// Example MIDI assignments for Behringer X-Touch
    fn x_touch_full() -> Self {
        ControllerAssignments {
            banks: vec![
                FaderBank {
                    name: Some("CH 1-8".to_string()),
                    faders: (1..=8).map(|i| format!("Channel {}", i)).collect(),
                },
                FaderBank {
                    name: Some("CH 9-16".to_string()),
                    faders: (9..=16).map(|i| format!("Channel {}", i)).collect(),
                },
                FaderBank {
                    name: Some("CH 17-24".to_string()),
                    faders: (17..=24).map(|i| format!("Channel {}", i)).collect(),
                },
                FaderBank {
                    name: Some("CH 25-32".to_string()),
                    faders: (25..=32).map(|i| format!("Channel {}", i)).collect(),
                },
                FaderBank {
                    name: Some("CH 33-40".to_string()),
                    faders: (33..=40).map(|i| format!("Channel {}", i)).collect(),
                },
                FaderBank {
                    name: Some("AUX 1-8".to_string()),
                    faders: (1..=8).map(|i| format!("Aux {}", i)).collect(),
                },
                FaderBank {
                    name: Some("BUS 1-8".to_string()),
                    faders: (1..=8).map(|i| format!("Bus {}", i)).collect(),
                },
                FaderBank {
                    name: Some("BUS 9-16".to_string()),
                    faders: (9..=16).map(|i| format!("Bus {}", i)).collect(),
                },
                FaderBank {
                    name: Some("MAIN".to_string()),
                    faders: (1..=4).map(|i| format!("Main {}", i)).collect(),
                },
                FaderBank {
                    name: Some("MATRIX".to_string()),
                    faders: (1..=8).map(|i| format!("Matrix {}", i)).collect(),
                },
                FaderBank {
                    name: Some("DCA 1-8".to_string()),
                    faders: (1..=8).map(|i| format!("DCA {}", i)).collect(),
                },
                FaderBank {
                    name: Some("DCA 9-16".to_string()),
                    faders: (9..=16).map(|i| format!("DCA {}", i)).collect(),
                },
            ],
            fader_buttons: vec!["Rec".to_string(), "Solo".to_string(), "Mute".to_string()],
            fixed_faders: HashMap::new(),
            fixed_buttons: HashMap::from([
                (46, "Previous Bank".to_string()),
                (47, "Next Bank".to_string()),
            ]),
        }
    }
}

impl MidiDefinition {
    /// Example MIDI definition for Behringer X-Touch
    fn x_touch_full() -> Self {
        // TODO: Add touch 104...112
        let channel_buttons = ["Rec", "Solo", "Mute", "Select", "Encoder Push"];

        let mut faders: Vec<MidiFader> = (0..8)
            .map(|ch| MidiFader {
                channel: ch,
                buttons: channel_buttons
                    .iter()
                    .enumerate()
                    .map(|(btn, &name)| MidiButton {
                        channel: ch,
                        key: ch + btn as u8 * 8,
                        description: Some(name.to_string()),
                    })
                    .collect(),
                description: Some(format!("Channel {}", ch + 1)),
            })
            .collect();

        faders.push(MidiFader {
            channel: 8.into(),
            buttons: vec![],
            description: Some("Master Fader".to_string()),
        });

        let buttons = vec![
            // Encoder Assign
            MidiButton {
                channel: 0,
                key: 40,
                description: Some("Track".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 42,
                description: Some("Pan".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 44,
                description: Some("EQ".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 41,
                description: Some("Send".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 43,
                description: Some("Plug-In".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 45,
                description: Some("Inst".to_string()),
            },
            // LCD Display
            MidiButton {
                channel: 0,
                key: 52,
                description: Some("Name/Value".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 53,
                description: Some("SMTPE".to_string()),
            },
            // Fader Assign
            MidiButton {
                channel: 0,
                key: 51,
                description: Some("Global View".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 62,
                description: Some("MIDI Tracks".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 63,
                description: Some("Inputs".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 64,
                description: Some("Audio Tracks".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 65,
                description: Some("Audio Inst".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 66,
                description: Some("Aux".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 67,
                description: Some("Buses".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 68,
                description: Some("Outputs".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 69,
                description: Some("User".to_string()),
            },
            // Main Fader
            MidiButton {
                channel: 0,
                key: 50,
                description: Some("Flip".to_string()),
            },
            // Function
            MidiButton {
                channel: 0,
                key: 54,
                description: Some("F1".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 55,
                description: Some("F2".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 56,
                description: Some("F3".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 57,
                description: Some("F4".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 58,
                description: Some("F5".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 59,
                description: Some("F6".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 60,
                description: Some("F7".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 61,
                description: Some("F8".to_string()),
            },
            // Modify
            MidiButton {
                channel: 0,
                key: 70,
                description: Some("Shift".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 71,
                description: Some("Option".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 72,
                description: Some("Ctrl".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 73,
                description: Some("Alt".to_string()),
            },
            // Automation
            MidiButton {
                channel: 0,
                key: 74,
                description: Some("Read/Off".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 75,
                description: Some("Write".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 76,
                description: Some("Trim".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 77,
                description: Some("Touch".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 78,
                description: Some("Latch".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 79,
                description: Some("Group".to_string()),
            },
            // Utility
            MidiButton {
                channel: 0,
                key: 80,
                description: Some("Save".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 81,
                description: Some("Undo".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 82,
                description: Some("Cancel".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 83,
                description: Some("Enter".to_string()),
            },
            // Transport
            MidiButton {
                channel: 0,
                key: 84,
                description: Some("Marker".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 85,
                description: Some("Nudge".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 86,
                description: Some("Cycle".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 87,
                description: Some("Drop".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 88,
                description: Some("Replace".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 89,
                description: Some("Click".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 90,
                description: Some("Solo".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 91,
                description: Some("Rewind".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 92,
                description: Some("Forward".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 93,
                description: Some("Stop".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 94,
                description: Some("Play".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 95,
                description: Some("Record".to_string()),
            },
            // Fader Bank
            MidiButton {
                channel: 0,
                key: 46,
                description: Some("Bank Left".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 47,
                description: Some("Bank Right".to_string()),
            },
            // Channel
            MidiButton {
                channel: 0,
                key: 48,
                description: Some("Channel Left".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 49,
                description: Some("Channel Right".to_string()),
            },
            // Navigation
            MidiButton {
                channel: 0,
                key: 96,
                description: Some("Up".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 97,
                description: Some("Down".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 98,
                description: Some("Left".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 99,
                description: Some("Right".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 100,
                description: Some("Enter".to_string()),
            },
            MidiButton {
                channel: 0,
                key: 101,
                description: Some("Scrub".to_string()),
            },
        ];

        MidiDefinition { faders, buttons }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            faders: [
                FaderAssignment {
                    osc: "ch.1.fdr".to_string(),
                },
                FaderAssignment {
                    osc: "ch.2.fdr".to_string(),
                },
                FaderAssignment {
                    osc: "ch.3.fdr".to_string(),
                },
                FaderAssignment {
                    osc: "ch.4.fdr".to_string(),
                },
                FaderAssignment {
                    osc: "ch.5.fdr".to_string(),
                },
                FaderAssignment {
                    osc: "ch.6.fdr".to_string(),
                },
                FaderAssignment {
                    osc: "ch.7.fdr".to_string(),
                },
                FaderAssignment {
                    osc: "ch.8.fdr".to_string(),
                },
            ],
            master: FaderAssignment {
                osc: "dca.1.fdr".to_string(),
            },
            console: ConsoleSettings {
                ip: "127.0.0.1".to_string(),
                port: 2223,
            },
            midi: ControllerSettings {
                input: "X-Touch".to_string(),
                output: "X-Touch".to_string(),
                assignments: ControllerAssignments::x_touch_full(),
            },
            midi_definition: MidiDefinition::x_touch_full(),
            mqtt: MqttSettings {
                host: "localhost".to_string(),
                port: 1883,
            },
        }
    }
}

impl Settings {
    pub fn new() -> Result<Self, figment::Error> {
        // as an example serialize and print default settings as json with spaces and newlines
        println!("{}", serde_yaml::to_string(&Settings::default()).unwrap());

        let settings: Settings = Figment::new()
            .merge(figment::providers::Serialized::defaults(Settings::default()))
            .merge(figment::providers::Yaml::file("config.yml"))
            .merge(figment::providers::Env::prefixed("WING_").split("_"))
            .extract()?;

        debug!("Loaded settings: {:#?}", settings);

        Ok(settings)
    }
}
