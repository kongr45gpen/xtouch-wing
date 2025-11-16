//! Common data types

#[derive(Debug, Clone, PartialEq)]
pub struct Fader {
	pub last_value: f32,
	pub osc_name: String,
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
        self.last_value
    }
}

impl ReadBool for Bool {
    fn read(&self) -> bool {
        self.last_value
    }
}
