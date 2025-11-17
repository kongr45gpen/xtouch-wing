//! MIDI controller wrapper for the X-Touch

use anyhow::Result;
use log::info;
use midir::{MidiInput, MidiInputConnection, MidiOutput};
use midly::live::LiveEvent;
use log::debug;

/// Simple controller owning a MIDI input and output handle.
pub struct Controller {
	pub input: MidiInputConnection<()>,
	pub output: MidiOutput,
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
        let input_port = ports.iter()
            .find(|p| input.port_name(p).ok().as_deref() == Some(input_name))
            .ok_or_else(|| anyhow::anyhow!("MIDI input port '{}' not found", input_name))?;

        let output_port = output.ports().iter()
            .find(|p| output.port_name(p).ok().as_deref() == Some(output_name))
            .ok_or_else(|| anyhow::anyhow!("MIDI output port '{}' not found", output_name))?;

        let input_connection =input.connect(input_port, "xtouch-wing-input", midi_callback, ())?;

        info!("MIDI input '{}' and output '{}' connected",
            input_name, output_name);

		Ok(Self { input: input_connection, output })
	}
}

fn midi_callback(timestamp_us: u64, message: &[u8], _: &mut ()) {
    // println!("MIDI message at {} us: {:?}", timestamp_us, message);
    let event = LiveEvent::parse(message);
    debug!("MIDI event at {} us: {:?}", timestamp_us, event);
}