//! PCM Chip Emulation
//!
//! Top-level emulation of the Model 2 PCM audio chip, including
//! register access and sample RAM.

use crate::channel::{Channel, PlaybackState};
use crate::{NUM_CHANNELS, Sample};

/// Size of PCM sample RAM in bytes (512 KB)
pub const PCM_RAM_SIZE: usize = 512 * 1024;

/// The PCM audio chip
pub struct PcmChip {
    /// PCM sample RAM
    pub ram: Box<[u8; PCM_RAM_SIZE]>,

    /// All voice channels
    pub channels: [Channel; NUM_CHANNELS],
}

impl PcmChip {
    pub fn new() -> Self {
        Self {
            ram: Box::new([0u8; PCM_RAM_SIZE]),
            channels: std::array::from_fn(|_| Channel::new()),
        }
    }

    /// Read a byte from PCM RAM
    pub fn read_ram(&self, addr: u32) -> u8 {
        let idx = (addr as usize) & (PCM_RAM_SIZE - 1);
        self.ram[idx]
    }

    /// Write a byte to PCM RAM
    pub fn write_ram(&mut self, addr: u32, value: u8) {
        let idx = (addr as usize) & (PCM_RAM_SIZE - 1);
        self.ram[idx] = value;
    }

    /// Key-on: start playback on a channel
    pub fn key_on(&mut self, ch: usize) {
        if ch < NUM_CHANNELS {
            let c = &mut self.channels[ch];
            c.current_addr = c.start_addr << 16;
            c.state = PlaybackState::Playing;
        }
    }

    /// Key-off: stop playback on a channel
    pub fn key_off(&mut self, ch: usize) {
        if ch < NUM_CHANNELS {
            self.channels[ch].state = PlaybackState::Stopped;
        }
    }

    /// Generate one stereo output sample by mixing all active channels.
    /// `clocks` is the number of chip clocks elapsed (used for timing if needed).
    pub fn clock(&mut self) -> Sample {
        let mut left: i32 = 0;
        let mut right: i32 = 0;

        for ch in self.channels.iter_mut() {
            if !ch.is_active() {
                continue;
            }

            let sample_idx = (ch.current_addr >> 16) as usize;
            let raw = if sample_idx < PCM_RAM_SIZE {
                // Treat sample as 8-bit signed
                (ch.ram_read_placeholder(sample_idx) as i8) as i32
            } else {
                0
            };

            // Apply volume (scale to 16-bit)
            left += (raw * ch.vol_left as i32) >> 3;
            right += (raw * ch.vol_right as i32) >> 3;

            ch.step();
        }

        Sample {
            left: left.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            right: right.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        }
    }
}

impl Default for PcmChip {
    fn default() -> Self {
        Self::new()
    }
}

// Temporary helper — in real code, pass a reference to PCM RAM into Channel or use the chip's RAM
trait RamRead {
    fn ram_read_placeholder(&self, idx: usize) -> u8;
}

impl RamRead for Channel {
    fn ram_read_placeholder(&self, _idx: usize) -> u8 {
        // This will be wired up to PcmChip::ram in later refactoring
        0
    }
}
