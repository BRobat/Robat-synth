//! Audio device handling (CoreAudio on macOS via cpal).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use engine::{new_engine, Session};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Most channels we will open; 16 = eight stereo pairs (BlackHole 16ch).
const MAX_CHANNELS: u16 = 16;
const PREFERRED_BUFFER: u32 = 256;

pub struct AudioOut {
    _backend: Backend,
    pub device: String,
    pub channels: usize,
    pub sample_rate: u32,
}

enum Backend {
    Stream(#[allow(dead_code)] cpal::Stream),
    /// No device: a thread keeps the engine (and the visuals) running.
    Silent(Arc<AtomicBool>),
}

impl Drop for Backend {
    fn drop(&mut self) {
        if let Backend::Silent(stop) = self {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

pub const SILENT_DEVICE: &str = "silent clock";

/// Runs the engine in real time without an audio device.
pub fn open_silent(session: &mut Session) -> AudioOut {
    const RATE: u32 = 48_000;
    const FRAMES: usize = 512;
    let (handle, mut core) = new_engine(RATE as f32);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        let mut buf = vec![0.0; FRAMES * 2];
        let start = Instant::now();
        let mut rendered = 0u64;
        while !flag.load(Ordering::Relaxed) {
            core.process(&mut buf, 2);
            rendered += FRAMES as u64;
            let due = start + Duration::from_secs_f64(rendered as f64 / RATE as f64);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
    });
    session.attach(handle, RATE as f32);
    AudioOut { _backend: Backend::Silent(stop), device: SILENT_DEVICE.into(), channels: 2, sample_rate: RATE }
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|desc| desc.name().to_string()).unwrap_or_else(|_| "unknown device".into())
}

/// Output devices with their maximum float channel count.
pub fn list_devices() -> Vec<(String, u16)> {
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else { return Vec::new() };
    devices
        .map(|d| {
            let max = d
                .supported_output_configs()
                .map(|cs| cs.filter(|c| c.sample_format() == cpal::SampleFormat::F32).map(|c| c.channels()).max().unwrap_or(0))
                .unwrap_or(0);
            (device_name(&d), max)
        })
        .filter(|(_, ch)| *ch > 0)
        .collect()
}

/// Opens `name` (or the system default), creates a fresh engine for it and
/// uploads the session's project into it.
pub fn open(name: Option<&str>, session: &mut Session) -> Result<AudioOut, String> {
    let host = cpal::default_host();
    let device = match name {
        Some(n) => host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| device_name(d) == n)
            .ok_or_else(|| format!("device '{n}' not found"))?,
        None => host.default_output_device().ok_or("no output device")?,
    };
    let device_label = device_name(&device);
    let default_rate = device.default_output_config().map(|c| c.sample_rate()).unwrap_or(48_000);

    let range = device
        .supported_output_configs()
        .map_err(|e| e.to_string())?
        .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
        .max_by_key(|c| (c.channels().min(MAX_CHANNELS), c.contains_rate(default_rate)))
        .ok_or("device has no float output format")?;
    let supported = range.try_with_sample_rate(default_rate).unwrap_or_else(|| range.with_max_sample_rate());
    let mut config = supported.config();
    config.channels = config.channels.min(MAX_CHANNELS);
    if let cpal::SupportedBufferSize::Range { min, max } = supported.buffer_size() {
        if (*min..=*max).contains(&PREFERRED_BUFFER) {
            config.buffer_size = cpal::BufferSize::Fixed(PREFERRED_BUFFER);
        }
    }

    let channels = config.channels as usize;
    let sample_rate = config.sample_rate;
    let (handle, mut core) = new_engine(sample_rate as f32);
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _| core.process(data, channels),
            |e| eprintln!("audio stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    session.attach(handle, sample_rate as f32);
    Ok(AudioOut { _backend: Backend::Stream(stream), device: device_label, channels, sample_rate })
}
