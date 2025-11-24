//! MIDI controller wrapper for the X-Touch

use core::f32;
use std::cell::{Cell, Ref, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::thread;

use anyhow::{Context, Result, anyhow};
use clap::error;
use tracing::{Level, debug, error, info, instrument, trace, warn};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use midly::PitchBend;
use midly::io::Write;
use midly::live::LiveEvent;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tracing_subscriber::field::debug;

use crate::data::{Fader, InternalButton, InternalFunction, PathType};
use crate::orchestrator::{Interface, Value, WriteProvider};
use crate::settings::{ControllerSettings, MidiDefinition};
use crate::utils::try_arc_new_cyclic;

const ASCII_TO_7SEGMENT: [Option<u8>; 128] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None, None, None, None, None, None, None,
    None, None, None, None,
    Some(0),  // space
    Some(59), // !
    Some(39), // "
    Some(35), // #
    Some(36), // $
    Some(37), // %
    Some(38), // &
    Some(39), // '
    Some(40), // (
    Some(41), // )
    Some(42), // *
    Some(43), // +
    Some(44), // ,
    Some(45), // -
    Some(46), // .
    Some(47), // /
    Some(48), // 0
    Some(49), // 1
    Some(50), // 2
    Some(51), // 3
    Some(52), // 4
    Some(53), // 5
    Some(54), // 6
    Some(55), // 7
    Some(56), // 8
    Some(57), // 9
    Some(34), // :
    Some(59), // ;
    Some(60), // <
    Some(61), // =
    Some(62), // >
    Some(63), // ?
    Some(38), // @
    Some(1),  // A
    Some(2),  // B
    Some(3),  // C
    Some(4),  // D
    Some(5),  // E
    Some(6),  // F
    Some(7),  // G
    Some(8),  // H
    Some(9),  // I
    Some(10), // J
    Some(11), // K
    Some(12), // L
    Some(13), // M
    Some(14), // N
    Some(15), // O
    Some(16), // P
    Some(17), // Q
    Some(18), // R
    Some(19), // S
    Some(20), // T
    Some(21), // U
    Some(22), // V
    Some(23), // W
    Some(24), // X
    Some(25), // Y
    Some(26), // Z
    Some(27), // [
    Some(28), // \
    Some(29), // ]
    Some(30), // ^
    Some(31), // _
    Some(32), // `
    Some(1),  // a
    Some(2),  // b
    Some(3),  // c
    Some(4),  // d
    Some(5),  // e
    Some(6),  // f
    Some(7),  // g
    Some(8),  // h
    Some(9),  // i
    Some(10), // j
    Some(11), // k
    Some(12), // l
    Some(13), // m
    Some(14), // n
    Some(15), // o
    Some(16), // p
    Some(17), // q
    Some(18), // r
    Some(19), // s
    Some(20), // t
    Some(21), // u
    Some(22), // v
    Some(23), // w
    Some(24), // x
    Some(25), // y
    Some(26), // z
    Some(27), // {
    Some(28), // |
    Some(29), // }
    Some(31), // ~
    Some(0),  // DEL
];

const WING_TO_XTOUCH_COLOR: [u8; 13] = [
    0, 7, 6, 4, 7, 2, 2, 3, 3, 1, 1, 5, 5
];

/// Simple controller owning a MIDI input and output handle.
pub struct Controller {
    pub input: Arc<std::sync::Mutex<MidiInputConnection<(Weak<Mutex<Controller>>, Handle)>>>,
    pub output: Arc<std::sync::Mutex<MidiOutputConnection>>,

    interface: Arc<Mutex<Option<Interface>>>,

    current_bank: usize,
    banks: Vec<Vec<Fader>>,
    bank_names: Vec<Option<String>>,
    buttons: HashMap<u32, InternalButton>,

    cached_colours: [u8; 8],
}

impl Controller {
    /// Create a new MIDI controller and initialise connections
    pub fn new(
        midi_settings: &ControllerSettings,
        midi_definition: &MidiDefinition,
    ) -> Result<Arc<Mutex<Self>>> {
        try_arc_new_cyclic(|weak| {
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

            // Wrap connect errors into anyhow so we don't require the backend error
            // types to be `Sync` for the `?` operator.
            let input_connection = input
                .connect(
                    input_port,
                    "xtouch-wing-input",
                    midi_callback,
                    (weak.clone(), Handle::current()),
                )
                .map_err(|e| anyhow!("MIDI input connect failed: {}", e))?;

            let output_connection = output
                .connect(output_port, "xtouch-wing-output")
                .map_err(|e| anyhow!("MIDI output connect failed: {}", e))?;

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
                        Fader::new_from_label(label).with_context(|| {
                            format!("Fader label '{}' in your configuration is invalid", label)
                        })
                    })
                    .collect::<Result<Vec<Fader>>>()?;

                banks.push(faders);
            }

            let buttons = midi_settings
                .assignments
                .fixed_buttons
                .iter()
                .map(|(index, label)| {
                    let button = InternalButton::new_from_label(label).with_context(|| {
                        format!("Button label '{}' in your configuration is invalid", label)
                    })?;

                    Ok((*index, button))
                })
                .collect::<Result<HashMap<u32, InternalButton>>>()?;

            Ok(Mutex::new(Self {
                input: Arc::new(std::sync::Mutex::new(input_connection)),
                output: Arc::new(std::sync::Mutex::new(output_connection)),
                interface: Arc::new(Mutex::new(None)),
                current_bank: 0,
                banks: banks,
                bank_names: midi_settings
                    .assignments
                    .banks
                    .iter()
                    .map(|b| b.name.clone())
                    .collect(),
                buttons: buttons,
                cached_colours: [7; _],
            }))
        })
    }

    #[instrument(name = "midi_set_fader", level = Level::DEBUG, skip(self, fader, value))]
    pub async fn process_fader_input(
        &mut self,
        fader_index: usize,
        fader: &Fader,
        path: PathType,
        value: &Value,
    ) -> Result<()> {
        match path {
            PathType::Fader => {
                if let Value::Float(db) = value {
                    let midi_value: f64 = Fader::db_to_float((*db) as f64);

                    debug!(fader_index, db = ?db, val = ?midi_value, "Setting fader value");

                    let ev = LiveEvent::Midi {
                        channel: (fader_index as u8).into(),
                        message: midly::MidiMessage::PitchBend {
                            // TODO: Handle 1.0 max value
                            bend: PitchBend::from_f64(midi_value * 2.0 - 1.0),
                        },
                    };

                    let mut buf = Vec::with_capacity(3);
                    ev.write(&mut buf)
                        .map_err(|e| anyhow!("MIDI write fail {}", e))?;
                    // synchronous context: use blocking_lock to acquire the Tokio mutex
                    self.send_midi(&buf)?;
                } else {
                    warn!("Expected float value for fader, got {:?}", value);
                }
            }
            PathType::ScribbleColour => {
                if let Value::Int(colour_index) = value {
                    debug!(fader_index, scribble_colour = colour_index, "Setting fader scribble colour");
                    let wing_color = WING_TO_XTOUCH_COLOR
                        .get(*colour_index as usize)
                        .copied()
                        .unwrap_or(7);

                    self.cached_colours[fader_index] = wing_color;
                    self.send_colours().await;
                } else {
                    warn!("Expected int value for scribble colour, got {:?}", value);
                }
            }
            PathType::ScribbleName => {
                if let Value::Str(name) = value {
                    debug!(fader_index, scribble_name = name.as_str(), "Setting fader scribble name");
                    self.set_lcd_text(name, fader_index as u8).await;
                } else {
                    warn!("Expected string value for scribble name, got {:?}", value);
                }
            }
            _ => {}
        }

        Ok(())
    }

    pub async fn process_osc_input(&mut self, osc_addr: &str, value: &Value) -> Result<()> {
        let faders = &self
            .banks
            .get(self.current_bank)
            .ok_or_else(|| anyhow::anyhow!("Current bank {} not found", self.current_bank))?;

        let faders = (*faders).clone();

        for (index, fader) in faders.iter().enumerate() {
            if let Some(path_type) = fader.path_matches(osc_addr) {
                self.process_fader_input(index, fader, path_type, value).await?;
            }
        }

        Ok(())
    }

    async fn refresh_bank(&self) -> Result<()> {
        info!("Hydrating bank {} buttons & faders", self.current_bank);

        let faders = self
            .banks
            .get(self.current_bank)
            .ok_or_else(|| anyhow::anyhow!("Bank {} not on list", self.current_bank))?;

        let interface_guard = self
                .interface
                .lock()
                .await;
        let interface = interface_guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Interface not set"))?;

        for (index, fader) in faders.iter().enumerate() {
            let osc_path = fader.get_osc_path(PathType::Fader);

            let value = interface
                .request_value_notification_checked(&osc_path, false)
                .await;

            if let Err(e) = value {
                warn!(
                    "OSC value for {} not found during bank refresh: {}",
                    osc_path, e
                );
            }

            interface
                .request_value_notification(&fader.get_osc_path(PathType::ScribbleColour), false)
                .await;

            interface
                .request_value_notification(&fader.get_osc_path(PathType::ScribbleName), false)
                .await;
        }

        drop(interface_guard);

        self.refresh_all_button_leds().await;

        self.write_text_to_main_display(
            self.bank_names
                .get(self.current_bank)
                .and_then(|name| name.as_deref())
                .unwrap_or(""),
        ).await;

        self.request_meters().await;

        Ok(())
    }

    async fn get_function_button_lit(&self, function: &InternalFunction) -> Result<bool> {
        let mut result: anyhow::Result<_>;

        match function {
            InternalFunction::NextBank => {
                result = Ok(self.current_bank + 1 < self.banks.len());
            },
            InternalFunction::PreviousBank => {
                result = Ok(self.current_bank > 0);
            },
        }

        result.with_context(|| format!("While checking function LED {:?}", function))
    }

    async fn refresh_button_led(&self, button: u32) {
        if let Some(internal_button) = self.buttons.get(&button) {
            let lit = self.get_function_button_lit(&internal_button.function).await;

            if let Err(e) = lit {
                warn!("Failed to get button LED state for button {}: {}", button, e);
                return;
            }

            let lit = lit.unwrap();

            let midi_value = if lit { 127 } else { 0 };

            let ev = LiveEvent::Midi {
                channel: 0.into(),
                message: midly::MidiMessage::NoteOn {
                    key: (button as u8).into(),
                    vel: midi_value.into(),
                },
            };

            let mut buf = Vec::with_capacity(3);
            ev.write(&mut buf)
                .map_err(|e| anyhow!("MIDI write fail {}", e))
                .unwrap();
            if let Err(e) = self.send_midi(&buf) {
                warn!("Failed to send MIDI for button {}: {}", button, e);
            }
        } else {
            // ...
        }
    }

    async fn refresh_all_button_leds(&self) {
        // TODO: Cache LED status and don't update if not necessary
        for button in self.buttons.keys() {
            self.refresh_button_led(*button).await;
        }
    }

    /// Clear all button LEDs (set to 0).
    pub async fn clean_buttons(&self) {
        for note in 0..115 {
            let ev = LiveEvent::Midi {
                channel: 0.into(),
                message: midly::MidiMessage::NoteOn {
                    key: (note as u8).into(),
                    vel: 0.into(),
                },
            };

            let mut buf = Vec::with_capacity(3);
            ev.write(&mut buf).unwrap();
            if let Err(e) = self.send_midi(&buf) {
                warn!("Failed to clear button {}: {}", note, e);
            }
        }
    }

    /// Send the current colours, as stored in the cache, to the controller. This does not
    /// update or request OSC values.
    async fn send_colours(&self) {
        let c = &self.cached_colours;

        let sysex = [
            0xF0, 0x00, 0x00, 0x66, 0x14, 0x72,
            c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7],
            0xF7,
        ];

        if let Err(e) = self.send_midi(&sysex) {
            warn!("Failed to send colour sysex: {}", e);
        }
    }

    async fn set_lcd_text(&self, text: &str, disp: u8) {
        const MAX_LEN: u8 = 7;
        const NUM_DISPLAYS: u8 = 8;

        if disp >= NUM_DISPLAYS {
            warn!("Invalid display index {:?}", disp);
            return;
        }

        let (row1_str, row2_str) = if text.contains(' ') && text.chars().count() <= (MAX_LEN as usize) * 2 {
            let mut parts = text.splitn(2, ' ');
            (
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            )
        } else {
            let mut it = text.chars();
            let a: String = it.by_ref().take(MAX_LEN as usize).collect();
            let b: String = it.take(MAX_LEN as usize).collect();
            (a, b)
        };

        fn pad(s: &str, max_len: usize) -> Vec<u8> {
            let mut bytes = s.bytes().collect::<Vec<u8>>();
            while bytes.len() < max_len {
                bytes.push(b' ');
            }
            bytes
        }

        let row1 = pad(&row1_str, MAX_LEN as usize);
        let row2 = pad(&row2_str, MAX_LEN as usize);
        let offset1 = disp.wrapping_mul(MAX_LEN);
        let offset2 = offset1.wrapping_add(NUM_DISPLAYS.wrapping_mul(MAX_LEN));

        let mut sysex1: Vec<u8> = [0xF0, 0x00, 0x00, 0x66, 0x14, 0x12, offset1].to_vec();
        sysex1.extend_from_slice(&row1);
        sysex1.push(0xF7);

        let mut sysex2: Vec<u8> = [0xF0, 0x00, 0x00, 0x66, 0x14, 0x12, offset2].to_vec();
        sysex2.extend_from_slice(&row2);
        sysex2.push(0xF7);

        if let Err(e) = self.send_midi(&sysex1) {
            warn!("Failed to write to display {} row1: {}", disp, e);
        }

        if let Err(e) = self.send_midi(&sysex2) {
            warn!("Failed to write to display {} row2: {}", disp, e);
        }
    }

    async fn do_function(&mut self, function: InternalFunction) -> Result<()> {
        let mut result;

        match function {
            InternalFunction::NextBank => {
                self.current_bank = (self.current_bank + 1) % self.banks.len();
                result = self.refresh_bank().await;
            }
            InternalFunction::PreviousBank => {
                if self.current_bank == 0 {
                    self.current_bank = self.banks.len() - 1;
                } else {
                    self.current_bank -= 1;
                }
                result = self.refresh_bank().await;
            }
        }

        result.with_context(|| format!("While executing function {:?}", function))
    }

    async fn write_text_to_main_display(&self, text: &str) {
        let display_cc = (64..=75).rev().collect::<Vec<u8>>();

        let text = text.chars().take(display_cc.len()).collect::<String>();

        // An offset to discard the first two characters because they are too far away on
        // the display
        let mut text_offset = 2;
        if text.len() > display_cc.len() - 2 {
            text_offset = 0;
        }

        // We iterate over the entire display to clear any digits that may have been left
        // from before
        for (i, cc) in display_cc.iter().enumerate() {
            let index = i.checked_sub(text_offset);
            let ch = match index {
                Some(idx) => text.chars().nth(idx).unwrap_or(' '),
                None => ' ',
            };
            let midi_value = ASCII_TO_7SEGMENT
                .get(ch as usize)
                .and_then(|v| *v);

            if let Some(midi_value) = midi_value {
                let ev = LiveEvent::Midi {
                    channel: 0.into(),
                    message: midly::MidiMessage::Controller {
                        controller: display_cc[i].into(),
                        value: midi_value.into(),
                    },
                };

                let mut buf = Vec::with_capacity(3);
                ev.write(&mut buf).unwrap();
                if let Err(e) = self.send_midi(&buf) {
                    warn!("Failed to write to main display: {}", e);
                }
            }
        }
    }

    fn send_midi(&self, data: &[u8]) -> Result<()> {
        trace!(?data, "MIDI output");

        match self.output.lock() {
            Ok(mut conn) => conn.send(data).map_err(|e| anyhow!("MIDI send failed: {}", e)),
            Err(e) => Err(anyhow!("Failed to lock MIDI output mutex: {:?}", e)),
        }
    }

    async fn request_meters(&self) {
        let bank = match self.banks.get(self.current_bank) {
            Some(b) => b,
            None => {
                error!("Current bank {} not found when requesting meters", self.current_bank);
                return;
            }
        };

        let meters = bank
            .iter()
            .filter_map(|fader| {
                fader.get_meter().clone()
            })
            .collect::<Vec<_>>();

        let interface = self.interface.lock().await;

        if interface.is_none() {
            error!("Interface not set when requesting meters");
            return;
        }

        let result = interface.as_ref().unwrap().subscribe_to_meters(meters).await;
        if let Err(e) = result {
            error!("Failed to subscribe to meters: {}", e);
        }
    }

    async fn send_meters(&self, values: Vec<Vec<f32>>) {
        // TODO: Handle non-existent meters!!!
        for (chan, channel_values) in values.iter().enumerate() {
            if chan >= 8 {
                warn!("Ignoring meter channel {} (only 0-7 supported)", chan);
                continue;
            }

            let level = channel_values.get(0).copied().unwrap_or(0.0);
            let level = level.clamp(0.0, 1.0);
            // Power scaling
            let level = level.powf(4.0);

            let channel_offset: u8 = (level * 15.0) as u8;

            let ev = LiveEvent::Midi {
                channel: 0.into(),
                message: midly::MidiMessage::ChannelAftertouch {
                    vel: (chan as u8 * 16 + channel_offset).into(),
                },
            };

            let mut buf = Vec::with_capacity(3);
            ev.write(&mut buf)
                .map_err(|e| anyhow!("MIDI write fail {}", e))
                .unwrap();
            if let Err(e) = self.send_midi(&buf) {
                warn!("Failed to send MIDI for meter channel {}: {}", chan, e);
            }
        }
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
            self.send_midi(&sysex)?;
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
                    self.send_midi(&buf)?;
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
                self.send_midi(&buf)?;
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
                self.send_midi(&buf)?;
                buf.clear();
            }

            // Encoders
            // CC 48-55, 56-63
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
                self.send_midi(&buf)?;
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
                self.send_midi(&sysex)?;
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
                self.send_midi(&buf)?;
                buf.clear();
            }

            clk += 1;
        }

        Ok(())
    }
}

impl WriteProvider for Arc<Mutex<Controller>> {
    fn write(&self, addr: &str, value: Value) -> anyhow::Result<()> {
        let controller = self.clone();
        let addr = addr.to_string();

        trace!(addr = addr.as_str(), ?value, "OSC input received");

        tokio::task::spawn(async move {
            let mut controller = controller.lock().await;

            if let Err(e) = controller.process_osc_input(&addr, &value).await {
                error!("Failed to process OSC input {} = {:?}: {}", addr, value, e);
            }
        });

        Ok(())
    }

    fn set_interface(&self, interface: Interface) {
        let controller = self.clone();

        tokio::task::spawn(async move {
            let controller = controller.lock().await;

            controller.interface.lock().await.replace(interface);

            if let Err(e) = controller.refresh_bank().await {
                error!("Failed to refresh bank on interface set: {}", e);
            }
        });
    }

    fn write_meter_values(&self, values: Vec<Vec<f32>>) -> anyhow::Result<()> {
        let controller = self.clone();

        tokio::task::spawn(async move {
            let controller = controller.lock().await;

            controller.send_meters(values).await;
        });

        Ok(())
    }
}

fn midi_callback(_timestamp_us: u64, bytes: &[u8], input: &mut (Weak<Mutex<Controller>>, Handle)) {
    let span = tracing::span!(tracing::Level::DEBUG, "midi_in");
    let _enter: tracing::span::Entered<'_> = span.enter();

    let event = LiveEvent::parse(bytes);
    debug!(bytes, ?event, "MIDI input");

    let (controller, handle) = input;

    let controller = match controller.upgrade() {
        Some(c) => c,
        None => {
            error!("MIDI callback called but controller not initialised");
            return;
        }
    };

    let mut controller_lock = controller.blocking_lock();

    match event {
        Ok(LiveEvent::Midi { channel, message }) => {
            match message {
                midly::MidiMessage::PitchBend { bend } => {
                    let fader_index = channel.as_int() as usize;
                    let faders = &controller_lock
                        .banks
                        .get(controller_lock.current_bank)
                        .expect("Current bank not found");

                    if let Some(fader) = faders.get(fader_index) {
                        let db_value = Fader::float_to_db((bend.as_f64() + 1.0) / 2.0) as f32;

                        let osc_addr = fader.get_osc_path(PathType::Fader);
                        let interface = controller_lock.interface.clone();

                        handle.spawn(async move {
                            interface
                                .lock()
                                .await
                                .as_ref()
                                .unwrap()
                                .set_value(&osc_addr, Value::Float(db_value))
                                .await;
                        });

                        // Emit the message back as midi so that the console doesn't complain
                        if let Err(e) = controller_lock.send_midi(bytes) {
                            warn!("Failed to echo MIDI message: {}", e);
                        }
                    } else {
                        warn!("Fader index {} not found in current bank", fader_index);
                    }
                }
                midly::MidiMessage::NoteOn { key, vel } => {
                    let note = key.as_int() as u32;

                    if vel.as_int() == 0 {
                        // Button released
                        return;
                    } else if vel.as_int() != 127 {
                        warn!("I am not prepared to handle MIDI input velocities such as {} for note {}", vel.as_int(), key.as_int());
                        return;
                    }

                    let maybe_function = controller_lock
                        .buttons
                        .get(&note)
                        .map(|b| b.function.clone());

                    drop(controller_lock);

                    if let Some(function) = maybe_function {
                        let controller_for_spawn = controller.clone();
                        handle.spawn(async move {
                            if let Err(e) = controller_for_spawn.lock().await.do_function(function.clone()).await {
                                error!(
                                    "Failed to execute button function {:?}: {}",
                                    function, e
                                );
                            }
                        });
                    } else {
                        debug!("Unassigned Note On for key {}", note);
                    }
                    return;
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
