//! Output Mixer
//!
//! Accumulates samples from the PCM chip (and optionally the 68K audio CPU)
//! and provides a buffer suitable for the frontend audio output.

use crate::Sample;

/// Simple ring-buffer mixer / sample queue
pub struct Mixer {
    buffer: Vec<Sample>,
    write_pos: usize,
    read_pos: usize,
    capacity: usize,
}

impl Mixer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![Sample::default(); capacity],
            write_pos: 0,
            read_pos: 0,
            capacity,
        }
    }

    /// Push a sample into the ring buffer.
    /// Drops the sample silently if the buffer is full.
    pub fn push(&mut self, sample: Sample) {
        let next = (self.write_pos + 1) % self.capacity;
        if next != self.read_pos {
            self.buffer[self.write_pos] = sample;
            self.write_pos = next;
        }
    }

    /// Pop the next sample from the ring buffer.
    /// Returns silence if the buffer is empty.
    pub fn pop(&mut self) -> Sample {
        if self.read_pos == self.write_pos {
            return Sample::default();
        }
        let s = self.buffer[self.read_pos];
        self.read_pos = (self.read_pos + 1) % self.capacity;
        s
    }

    /// Number of samples currently available
    pub fn available(&self) -> usize {
        (self.write_pos + self.capacity - self.read_pos) % self.capacity
    }

    /// True if the buffer has no samples ready
    pub fn is_empty(&self) -> bool {
        self.read_pos == self.write_pos
    }
}
