//! PCM Channel State
//!
//! Represents a single PCM voice/channel on the Model 2 audio hardware.

/// Playback state of a single PCM channel
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Looping,
}

/// A single PCM channel
#[derive(Debug, Default)]
pub struct Channel {
    /// Current playback state
    pub state: PlaybackState,

    /// Start address in PCM sample RAM
    pub start_addr: u32,

    /// Current playback address (fixed-point, upper bits = sample index)
    pub current_addr: u32,

    /// Loop start address
    pub loop_addr: u32,

    /// End address in PCM sample RAM
    pub end_addr: u32,

    /// Playback pitch / frequency step (fixed-point)
    pub pitch: u32,

    /// Volume: left channel (0–255)
    pub vol_left: u8,

    /// Volume: right channel (0–255)
    pub vol_right: u8,

    /// Pan position (-128 to 127)
    pub pan: i8,
}

impl Channel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if this channel is currently active (playing or looping)
    pub fn is_active(&self) -> bool {
        matches!(self.state, PlaybackState::Playing | PlaybackState::Looping)
    }

    /// Advance the playback address by one pitch step.
    /// Returns true if the channel has reached or passed its end address.
    pub fn step(&mut self) -> bool {
        if !self.is_active() {
            return false;
        }

        self.current_addr = self.current_addr.wrapping_add(self.pitch);
        let sample_index = self.current_addr >> 16;

        if sample_index >= self.end_addr {
            match self.state {
                PlaybackState::Looping => {
                    // Wrap back to loop start
                    self.current_addr = self.loop_addr << 16;
                    false
                }
                _ => {
                    self.state = PlaybackState::Stopped;
                    true
                }
            }
        } else {
            false
        }
    }
}
