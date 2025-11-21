//! MIDI controller wrapper for the X-Touch

use core::f32;
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::Arc;
use std::thread;

use anyhow::anyhow;
use anyhow::{Context,Result};
use log::debug;
use log::info;
use log::warn;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use midly::PitchBend;
use midly::io::Write;
use midly::live::LiveEvent;
use tokio::sync::Mutex;

use crate::orchestrator::Value;
use crate::data::Fader;
use crate::data::PathType;
use crate::orchestrator::Interface;
use crate::orchestrator::WriteProvider;
use crate::settings::ControllerSettings;
use crate::settings::MidiDefinition;

/// Simple controller owning a MIDI input and output handle.
pub struct Controller {
    pub input: RefCell<MidiInputConnection<()>>,
    pub output: RefCell<MidiOutputConnection>,

    interface: Arc<Mutex<Option<Interface>>>,

    current_bank: usize,
    banks: Vec<Vec<Fader>>,
}

unsafe impl Send for Controller {}
unsafe impl Sync for Controller {}

static mut DANGEROUS_PTR: * const Controller = std::ptr::null();

impl Controller {
    /// Create a new MIDI controller and initialise connections
    pub fn new(
        midi_settings: &ControllerSettings,
        midi_definition: &MidiDefinition,
    ) -> Result<Self> {
        let input_name = &midi_settings.input;
        let output_name = &midi_settings.output;

        let input = MidiInput::new("X-Touch Wing IN")?;
        let output = MidiOutput::new("X-Touch Wing OUT")?;

        let ports = input.ports();
        let input_port = ports
            .iter()
            .find(|p| input.port_name(p).ok().as_deref() == Some(&input_name))
            .ok_or_else(|| anyhow::anyhow!("MIDI input port '{}' not found", input_name))?;

        let ports = output.ports();
        let output_port = ports
            .iter()
            .find(|p| output.port_name(p).ok().as_deref() == Some(&output_name))
            .ok_or_else(|| anyhow::anyhow!("MIDI output port '{}' not found", output_name))?;

        let input_connection = input.connect(input_port, "xtouch-wing-input", midi_callback, ())?;

        let output_connection = output.connect(output_port, "xtouch-wing-output")?;

        info!(
            "MIDI input '{}' and output '{}' connected",
            input_name, output_name
        );

        let mut banks = Vec::new();
        for bank in &midi_settings.assignments.banks {
            let faders = bank
                .faders
                .iter()
                .map(|label| {
                    Fader::new_from_label(label)
                        .with_context(|| format!("Failed to create fader from label '{}'", label))
                })
                .collect::<Result<Vec<Fader>>>()?;

            banks.push(faders);
        }

        let result = Ok(Self {
            input: RefCell::new(input_connection),
            output: RefCell::new(output_connection),
            interface: Arc::new(Mutex::new(None)),
            current_bank: 0,
            banks: banks,
        });

        result
    }

    pub fn process_fader_input(
        &self,
        fader_index: usize,
        fader: &Fader,
        path: PathType,
        value: &Value,
    ) -> Result<()> {
        match path {
            PathType::Fader => {
                if let Value::Float(db) = value {
                    let midi_value: f64 = Fader::db_to_float((*db) as f64);

                    let ev = LiveEvent::Midi {
                        channel: (fader_index as u8).into(),
                        message: midly::MidiMessage::PitchBend {
                            // TODO: Handle 1.0 max value
                            bend: PitchBend::from_f64(midi_value * 2.0 - 1.0),
                        },
                    };

                    let mut buf = Vec::with_capacity(3);
                    ev.write(&mut buf).map_err(|e| anyhow!("MIDI write fail {}", e))?;
                    self.output.try_borrow_mut()?.send(&buf)?;
                } else {
                    warn!("Expected float value for fader, got {:?}", value);
                }
            }
            _ => {}
        }

        Ok(())
    }

    pub fn process_osc_input(&self, osc_addr: &str, value: &Value) -> Result<()> {
        debug!("Processing OSC input {} = {:?}", osc_addr, value);

        let faders = &self
            .banks
            .get(self.current_bank)
            .ok_or_else(|| anyhow::anyhow!("Current bank {} not found", self.current_bank))?;

        let faders = (*faders).clone();

        for (index, fader) in faders.iter().enumerate() {
            if let Some(path_type) = fader.path_matches(osc_addr) {
                self.process_fader_input(index, fader, path_type, value)?;
            }
        }

        Ok(())
    }

    async fn refresh_bank(&self) -> Result<()> {
        debug!("Hydrating bank {} buttons & faders", self.current_bank);

        let faders = &self
            .banks
            .get(self.current_bank)
            .ok_or_else(|| anyhow::anyhow!("Bank {} not on list", self.current_bank))?;

        for (index, fader) in faders.iter().enumerate() {
            let osc_path = fader.get_osc_path(PathType::Fader);
            let value = self.interface
                .lock()
                .await
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Interface not set"))?
                .get_value(&osc_path)
                .await?;

            if let Some(value) = value {
                self.process_osc_input(&osc_path, &value)?;
            } else {
                warn!("OSC value for {} not found during bank refresh", osc_path);
            }
        }

        Ok(())
    }

    /// Runs a never-ending Vegas mode test pattern.
    pub async fn vegas_mode(&mut self, faders: bool) -> Result<()> {
        let mut clk = 0;

        {
            let max_len = 56 * 2;
            let message = b"Hello this is a test message from kongr45gpen!          Hello this is a test message from kongr45gpen!";

            // Text display
            let mut sysex: Vec<u8> = [
                0xF0, 0x00, 0x00, 0x66, 0x14, 0x12, 0x00, // Header
            ]
            .to_vec();
            sysex.extend_from_slice(&message[..max_len.min(message.len())]);
            sysex.push(0xF7);
            self.output.get_mut().send(&sysex)?;
        }

        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(1000 / 30)).await;

            let mut buf = Vec::new();

            // Pitch bends channels 0 - 8
            for channel in 0..9 {
                let value =
                    f32::sin(clk as f32 * 0.2 + channel as f32 / 9.0 * 2.0 * f32::consts::PI);

                let ev = LiveEvent::Midi {
                    channel: channel.into(),
                    message: midly::MidiMessage::PitchBend {
                        bend: PitchBend::from_f32(value),
                    },
                };

                ev.write(&mut buf).unwrap();
                if faders {
                    self.output.get_mut().send(&buf)?;
                }
                buf.clear();
            }

            // Notes 0-101 channel 0
            // keys = 0..102 and 113 and 114 and 115
            let keys = (0..102).chain(113..116);
            for key in keys {
                let vel = f32::sin(clk as f32 * 0.3 + key as f32 * 199.352);

                let vel = if vel > 0.2 { 127 } else { 0 };

                let ev = LiveEvent::Midi {
                    channel: 0.into(),
                    message: midly::MidiMessage::NoteOn {
                        key: key.into(),
                        vel: vel.into(),
                    },
                };

                ev.write(&mut buf).unwrap();
                self.output.get_mut().send(&buf)?;
                buf.clear();
            }

            // Meters
            // Notes 0-120 channel 1
            for chan in 0..8 {
                let level = f32::sin(-clk as f32 * 0.3 + chan as f32 / 9.0 * 2.0 * f32::consts::PI);
                // Map from -1..1 to 0..1
                let level = (level + 1.0) / 2.0;

                let channel_offset: u8 = (level * 15.0) as u8;

                let ev = LiveEvent::Midi {
                    channel: 0.into(),
                    message: midly::MidiMessage::ChannelAftertouch {
                        // key: (chan * 16 + channel_offset).into(),
                        vel: (chan * 16 + channel_offset).into(),
                    },
                };

                ev.write(&mut buf).unwrap();
                self.output.get_mut().send(&buf)?;
                buf.clear();
            }

            // Encoders
            // CC 48-55, 56-63
            // TODO: Investigate patterns. Currently it seems they have 4 patterns (no edge lights) + 4 patterns (with edge lights)
            for encoder in 0..8 {
                let value =
                    f32::sin(-clk as f32 * 0.02 + encoder as f32 * 0.02 * 2.0 * f32::consts::PI);
                // Map from -1..1 to 0..127
                let value = ((value + 1.0) / 2.0 * 127.0) as u8;

                let ev = LiveEvent::Midi {
                    channel: 0.into(),
                    message: midly::MidiMessage::Controller {
                        controller: (48 + encoder).into(),
                        value: value.into(),
                    },
                };

                ev.write(&mut buf).unwrap();
                self.output.get_mut().send(&buf)?;
                buf.clear();
            }

            {
                let colours = (0..8)
                    .map(|i| {
                        let c =
                            f32::sin(-clk as f32 * 0.1 + i as f32 * 0.2 * 2.0 * f32::consts::PI);
                        ((c + 1.0) / 2.0 * 7.0) as u8
                    })
                    .collect::<Vec<u8>>();

                let sysex = [
                    0xF0, 0x00, 0x00, 0x66, 0x14, 0x72, // Header
                    colours[0], colours[1], colours[2], colours[3], colours[4], colours[5],
                    colours[6], colours[7], // Colours
                    0xF7,
                ];

                // Scribble Strip Colours (sysex)
                self.output.get_mut().send(&sysex)?;
            }

            // 7-segment display
            // CC 96-107, 112-123
            // Actual CC 64-76
            // From right to left
            for cc in 64..76 {
                let value = f32::sin(-clk as f32 * 0.02 + cc as f32 * 0.01 * 2.0 * f32::consts::PI);
                // Map from -1..1 to 0..127
                let value = ((value + 1.0) / 2.0 * 127.0) as u8;

                // The display seems to be following a custom ASCII code
                // starting from letters + symbols + numbers, duplicated wrt the comma display
                let ev = LiveEvent::Midi {
                    channel: 0.into(),
                    message: midly::MidiMessage::Controller {
                        controller: cc.into(),
                        value: value.into(),
                    },
                };

                ev.write(&mut buf).unwrap();
                self.output.get_mut().send(&buf)?;
                buf.clear();
            }

            clk += 1;
        }

        Ok(())
    }
}

impl WriteProvider for Controller {
    fn write(&self, addr: &str, value: Value) -> anyhow::Result<()> {
        self.process_osc_input(addr, &value)
    }

    fn set_interface(&self, interface: Interface) {
        let old_interface = self.interface.clone();

        // TODO: What is this even
        let controller_ptr = (self as *const Controller) as usize;

        tokio::task::spawn(async move {
            old_interface.lock().await.replace(interface);
            // TODO: Very dangerous, extremely hacky code
            // UNSAFE: There is no excuse for this.

            unsafe {
                let controller_ptr: *const Controller = controller_ptr as *const Controller;
                let controller: &Controller = &*controller_ptr;

                if let Err(e) = controller.refresh_bank().await {
                    log::error!("Failed to refresh bank on interface set: {}", e);
                }
            }
        });

        unsafe {
            // TODO: Hope for the best...
            DANGEROUS_PTR = &*self;
        }
    }
}

fn midi_callback(timestamp_us: u64, bytes: &[u8], _: &mut ()) {
    // TODO: Hope for the best when recovering pointer

    let event = LiveEvent::parse(bytes);
    debug!("MIDI event at {} us: {:?}", timestamp_us, event);

    unsafe {
        let controller: &Controller = &*DANGEROUS_PTR;

        match event {
            Ok(LiveEvent::Midi { channel, message }) => {
                match message {
                    midly::MidiMessage::PitchBend { bend } => {
                        let fader_index = channel.as_int() as usize;
                        let faders = &controller
                            .banks
                            .get(controller.current_bank)
                            .expect("Current bank not found");

                        if let Some(fader) = faders.get(fader_index) {
                            let db_value =
                                Fader::float_to_db((bend.as_f64() + 1.0) / 2.0) as f32;


                            let osc_addr = fader.get_osc_path(PathType::Fader);
                            let interface = controller.interface.clone();

                            // TODO: This is incorrect
                            let rt = tokio::runtime::Runtime::new().unwrap();
                            
                            rt.spawn(async move {
                                interface.lock().await.as_ref().unwrap().set_value(&osc_addr, Value::Float(db_value)).await;
                            });

                            // Emit the message back as midi so that the console doesn't complain
                            controller.output.borrow_mut().send(bytes).unwrap();

                        } else {
                            warn!("Fader index {} not found in current bank", fader_index);
                        }
                    }
                    other => {
                        warn!("Unhandled MIDI message: {:?}", other);
                    }
                }
            }
            Ok(e) => {
                warn!("I am not equipped to understand this {:?} MIDI event", e);
            }
            Err(e) => {
                warn!("Failed to parse MIDI event: {}", e);
            }
        }
    }
}
