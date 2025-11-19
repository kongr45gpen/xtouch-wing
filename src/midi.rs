//! MIDI controller wrapper for the X-Touch

use core::f32;

use anyhow::Result;
use log::debug;
use log::info;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use midly::PitchBend;
use midly::io::Write;
use midly::live::LiveEvent;

use crate::console::Value;
use crate::orchestrator::WriteProvider;

/// Simple controller owning a MIDI input and output handle.
pub struct Controller {
    pub input: MidiInputConnection<()>,
    pub output: MidiOutputConnection,
}

impl Controller {
    /// Create a new MIDI controller and initialise connections
    pub fn new(input_name: &str, output_name: &str) -> Result<Self> {
        let input = MidiInput::new("X-Touch Wing IN")?;
        let output = MidiOutput::new("X-Touch Wing OUT")?;

        let ports = input.ports();
        let input_port = ports
            .iter()
            .find(|p| input.port_name(p).ok().as_deref() == Some(input_name))
            .ok_or_else(|| anyhow::anyhow!("MIDI input port '{}' not found", input_name))?;

        let ports = output.ports();
        let output_port = ports
            .iter()
            .find(|p| output.port_name(p).ok().as_deref() == Some(output_name))
            .ok_or_else(|| anyhow::anyhow!("MIDI output port '{}' not found", output_name))?;

        let input_connection = input.connect(input_port, "xtouch-wing-input", midi_callback, ())?;

        let output_connection = output.connect(output_port, "xtouch-wing-output")?;

        info!(
            "MIDI input '{}' and output '{}' connected",
            input_name, output_name
        );

        Ok(Self {
            input: input_connection,
            output: output_connection,
        })
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
            ].to_vec();
            sysex.extend_from_slice(&message[..max_len.min(message.len())]);
            sysex.push(0xF7);
            self.output.send(&sysex)?;
        }

        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(1000 / 30)).await;

            let mut buf = Vec::new();

            // Pitch bends channels 0 - 8
            for channel in 0..9 {
                let value = f32::sin(clk as f32 * 0.2 + channel as f32 / 9.0 * 2.0 * f32::consts::PI);

                let ev = LiveEvent::Midi {
                    channel: channel.into(),
                    message: midly::MidiMessage::PitchBend {
                        bend: PitchBend::from_f32(value),
                    },
                };

                ev.write(&mut buf).unwrap();
                if faders {
                    self.output.send(&buf)?;
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
                self.output.send(&buf)?;
                buf.clear();
            }

            // Meters
            // Notes 0-120 channel 1
            for chan in 0..8 {
                let level = f32::sin(- clk as f32 * 0.3 + chan as f32 / 9.0 * 2.0 * f32::consts::PI);
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
                self.output.send(&buf)?;
                buf.clear();
            }

            // Encoders
            // CC 48-55, 56-63
            // TODO: Investigate patterns. Currently it seems they have 4 patterns (no edge lights) + 4 patterns (with edge lights)
            for encoder in 0..8 {
                let value = f32::sin(- clk as f32 * 0.02 + encoder as f32 * 0.02 * 2.0 * f32::consts::PI);
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
                self.output.send(&buf)?;
                buf.clear();
            }


            {
                // let colour = f32::sin(clk as f32 * 0.1);

                // Create an array of 8 colours based on sine wave
                let colours = (0..8)
                    .map(|i| {
                        let c = f32::sin(-clk as f32 * 0.1 + i as f32 * 0.2 * 2.0 * f32::consts::PI);
                        ((c + 1.0) / 2.0 * 7.0) as u8
                    })
                    .collect::<Vec<u8>>();

                let sysex = [
                    0xF0, 0x00, 0x00, 0x66, 0x14, 0x72, // Header
                    colours[0], colours[1], colours[2], colours[3],
                    colours[4], colours[5], colours[6], colours[7], // Colours
                    0xF7,
                ];

                // let colour = ((colour + 1.0) / 2.0 * 7.0) as u8;
                // Scribble Strip Colours (sysex)
                self.output.send(&sysex)?;
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
                self.output.send(&buf)?;
                buf.clear();
            }

            clk += 1;
        }

        Ok(())
    }
}

impl WriteProvider for Controller {
    fn write(&self, addr: &str, value: Value) -> anyhow::Result<()> {
        unimplemented!("I was asked to write a value to the MIDI")
    }
}

fn midi_callback(timestamp_us: u64, message: &[u8], _: &mut ()) {
    // println!("MIDI message at {} us: {:?}", timestamp_us, message);
    let event = LiveEvent::parse(message);
    debug!("MIDI event at {} us: {:?}", timestamp_us, event);
}
