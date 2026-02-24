//! Model 2 PCM Audio Subsystem
//!
//! This crate emulates the PCM audio hardware of the Sega Model 2 arcade system.
//! The Model 2 uses a custom PCM chip for sound effects and music playback.

pub mod channel;
pub mod mixer;
pub mod pcm;

pub use mixer::Mixer;
pub use pcm::PcmChip;

/// Audio sample rate used for output (Hz)
pub const SAMPLE_RATE: u32 = 44100;

/// Number of PCM channels available on the hardware
pub const NUM_CHANNELS: usize = 32;

/// Audio output sample (stereo, 16-bit signed)
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub left: i16,
    pub right: i16,
}
