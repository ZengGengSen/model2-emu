//! Audio output via `cpal`.
//!
//! Spawns a background audio stream that pulls samples from a shared
//! ring-buffer filled by the emulator thread.

use cpal::{
    SampleFormat, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use model2_audio::mixer::Mixer;
use std::sync::{Arc, Mutex};

/// Number of samples in the shared ring buffer
const RING_CAPACITY: usize = 4096;

/// Handle to the running audio stream (drops → stream stops)
pub struct AudioOutput {
    _stream: Stream,
    /// Shared mixer – emulator thread calls `push()`, audio thread calls `pop()`
    pub mixer: Arc<Mutex<Mixer>>,
}

impl AudioOutput {
    /// Open the default audio device and start streaming.
    /// Returns `None` if no audio device is available (silently degraded).
    pub fn start() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;

        let config = device.default_output_config().ok()?;
        let sample_fmt = config.sample_format();
        let cfg: StreamConfig = config.into();

        log::info!(
            "Audio device: {}  channels: {}  sample_rate: {}  format: {:?}",
            device
                .description()
                .map(|d| d.name().to_string().clone())
                .unwrap_or_default(),
            cfg.channels,
            cfg.sample_rate,
            sample_fmt,
        );

        let mixer = Arc::new(Mutex::new(Mixer::new(RING_CAPACITY)));
        let mixer_cb = Arc::clone(&mixer);

        let channels = cfg.channels as usize;

        let stream = match sample_fmt {
            SampleFormat::F32 => build_stream::<f32>(&device, &cfg, mixer_cb, channels),
            SampleFormat::I16 => build_stream::<i16>(&device, &cfg, mixer_cb, channels),
            SampleFormat::U16 => build_stream::<u16>(&device, &cfg, mixer_cb, channels),
            _ => {
                log::warn!("Unsupported sample format {:?}, audio disabled", sample_fmt);
                return None;
            }
        }
        .ok()?;

        stream.play().ok()?;

        Some(AudioOutput {
            _stream: stream,
            mixer,
        })
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
    channels: usize,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: cpal::Sample + cpal::SizedSample + FromSample,
{
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut mx = mixer.lock().unwrap();
            // Fill the output buffer frame by frame
            let frames = data.len() / channels;
            for frame in 0..frames {
                let s = mx.pop();
                let l = T::from_sample_i16(s.left);
                let r = T::from_sample_i16(s.right);
                let base = frame * channels;
                data[base] = l;
                if channels > 1 {
                    data[base + 1] = r;
                }
                // Any additional channels get silence
                for ch in 2..channels {
                    data[base + ch] = T::from_sample_i16(0);
                }
            }
        },
        |err| log::error!("cpal stream error: {err}"),
        None,
    )
}

// ── Tiny conversion trait ─────────────────────────────────────────────────────

/// Convert an i16 PCM sample to the target cpal sample type.
pub trait FromSample: Sized {
    fn from_sample_i16(s: i16) -> Self;
}

impl FromSample for f32 {
    fn from_sample_i16(s: i16) -> Self {
        s as f32 / 32768.0
    }
}

impl FromSample for i16 {
    fn from_sample_i16(s: i16) -> Self {
        s
    }
}

impl FromSample for u16 {
    fn from_sample_i16(s: i16) -> Self {
        (s as i32 + 32768) as u16
    }
}
