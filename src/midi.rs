//! MIDI controller wrapper for the X-Touch

use core::f32;

use anyhow::Result;
use log::debug;
use log::info;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use midly::PitchBend;
use midly::live::LiveEvent;

/// Simple controller owning a MIDI input and output handle.
pub struct Controller {
    pub input: MidiInputConnection<()>,
    pub output: MidiOutputConnection,
}

impl Controller {
    /// Create a new `Controller` with default client names.
    ///
    /// This will initialize both a `MidiInput` and `MidiOutput` instance and
    /// return them wrapped in the `Controller` struct. It does not open any
    /// ports or create active connections; that can be done by the caller with
    /// the returned `input`/`output` handles.
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

    pub async fn vegas_mode(&mut self, enabled: bool) -> Result<()> {
        let mut clk = 0;

        loop {
            debug!("Vegas   {}", enabled);
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
                // self.output.send(&buf)?;
                buf.clear();
            }

            // Notes 0-101 channel 0
            for key in 0..102 {
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
            // Channel 0
            // Controllers 16-23
            // Notes 0-101

            clk += 1;
        }

        Ok(())
    }
}

fn midi_callback(timestamp_us: u64, message: &[u8], _: &mut ()) {
    // println!("MIDI message at {} us: {:?}", timestamp_us, message);
    let event = LiveEvent::parse(message);
    debug!("MIDI event at {} us: {:?}", timestamp_us, event);
}
