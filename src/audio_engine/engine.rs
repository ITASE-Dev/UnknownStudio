//! cpal output stream + mixer thread.
//!
//! Three parts:
//!  - `AudioEngine` (UI thread): sends commands, reads the playback clock.
//!  - Mixer thread: renders the timeline program into a ring buffer.
//!  - cpal callback: consumes the ring buffer and does no I/O whatsoever.
//!
//! The sound card's clock is the master clock; video follows it.

use crate::audio_engine::mixer::{AudioSegment, TimelineMixer};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// How far ahead of the callback the mixer renders.
const RING_TARGET_SECONDS: f64 = 0.35;
/// Stereo frames produced per pass.
const CHUNK_FRAMES: usize = 1024;

enum AudioCommand {
    SetProgram(Vec<AudioSegment>),
    Shutdown,
}

struct Shared {
    /// Interleaved samples in the device's channel layout.
    ring: Mutex<VecDeque<f32>>,
    playing: AtomicBool,
    /// Frames the callback consumed in this generation.
    consumed: AtomicU64,
    /// Bumped on every seek; the mixer discards stale audio by comparing it.
    generation: AtomicU64,
    /// Timeline second this generation started at (f64 bits).
    origin_bits: AtomicU64,
    volume_milli: AtomicU64,
}

impl Shared {
    fn origin(&self) -> f64 {
        f64::from_bits(self.origin_bits.load(Ordering::Acquire))
    }
}

pub struct AudioEngine {
    shared: Arc<Shared>,
    commands: Sender<AudioCommand>,
    sample_rate: u32,
    /// Last program sent; resending an identical one would drop open sources.
    program: Vec<AudioSegment>,
    /// cpal streams are not `Send`; this must stay alive on the UI thread.
    _stream: cpal::Stream,
}

impl AudioEngine {
    /// `None` when the machine has no usable output device — the app then runs
    /// silently on its own clock rather than failing to start.
    pub fn new() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let config = device.default_output_config().ok()?;
        let sample_format = config.sample_format();
        let config: cpal::StreamConfig = config.into();

        let sample_rate = config.sample_rate.0;
        let channels = config.channels as usize;

        let shared = Arc::new(Shared {
            ring: Mutex::new(VecDeque::new()),
            playing: AtomicBool::new(false),
            consumed: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            origin_bits: AtomicU64::new(0.0f64.to_bits()),
            volume_milli: AtomicU64::new(1000),
        });

        let stream = build_stream(&device, &config, sample_format, shared.clone(), channels)?;
        stream.play().ok()?;

        let (tx, rx) = mpsc::channel::<AudioCommand>();
        spawn_mixer(shared.clone(), rx, sample_rate, channels);

        Some(Self {
            shared,
            commands: tx,
            sample_rate,
            program: Vec::new(),
            _stream: stream,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Publishes the audible program if it changed.
    pub fn set_program(&mut self, program: Vec<AudioSegment>) {
        if program == self.program {
            return;
        }
        self.program = program.clone();
        let _ = self.commands.send(AudioCommand::SetProgram(program));
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared
            .volume_milli
            .store((volume.clamp(0.0, 2.0) * 1000.0) as u64, Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        self.shared.volume_milli.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    /// Restarts output at `second`, dropping everything already buffered.
    pub fn seek(&self, second: f32) {
        self.shared.playing.store(false, Ordering::Release);
        self.shared
            .origin_bits
            .store((second.max(0.0) as f64).to_bits(), Ordering::Release);
        self.shared.consumed.store(0, Ordering::Release);
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut ring) = self.shared.ring.lock() {
            ring.clear();
        }
    }

    pub fn play(&self) {
        self.shared.playing.store(true, Ordering::Release);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::Release);
    }

    /// Timeline second the sound card has reached — the master clock.
    pub fn position_seconds(&self) -> f32 {
        let consumed = self.shared.consumed.load(Ordering::Acquire);
        (self.shared.origin() + consumed as f64 / self.sample_rate as f64) as f32
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.commands.send(AudioCommand::Shutdown);
    }
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    shared: Arc<Shared>,
    channels: usize,
) -> Option<cpal::Stream> {
    let err_fn = |_err| {};

    match format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let filled = drain_ring(&shared, data.len(), |i, value| data[i] = value);
                    commit(&shared, channels, filled);
                },
                err_fn,
                None,
            )
            .ok(),
        cpal::SampleFormat::I16 => device
            .build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let filled = drain_ring(&shared, data.len(), |i, value| {
                        data[i] = (value * i16::MAX as f32) as i16;
                    });
                    commit(&shared, channels, filled);
                },
                err_fn,
                None,
            )
            .ok(),
        cpal::SampleFormat::U16 => device
            .build_output_stream(
                config,
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    let filled = drain_ring(&shared, data.len(), |i, value| {
                        data[i] = ((value * 0.5 + 0.5) * u16::MAX as f32) as u16;
                    });
                    commit(&shared, channels, filled);
                },
                err_fn,
                None,
            )
            .ok(),
        _ => None,
    }
}

/// Callback body: `try_lock` so the audio thread can never block, silence when
/// the mixer has not caught up.
fn drain_ring(shared: &Shared, len: usize, mut write: impl FnMut(usize, f32)) -> usize {
    let mut written = 0usize;

    if shared.playing.load(Ordering::Acquire) {
        if let Ok(mut ring) = shared.ring.try_lock() {
            while written < len {
                match ring.pop_front() {
                    Some(sample) => {
                        write(written, sample);
                        written += 1;
                    }
                    None => break,
                }
            }
        }
    }

    for i in written..len {
        write(i, 0.0);
    }
    written
}

fn commit(shared: &Shared, channels: usize, written: usize) {
    if written > 0 {
        shared
            .consumed
            .fetch_add((written / channels.max(1)) as u64, Ordering::AcqRel);
    }
}

fn spawn_mixer(
    shared: Arc<Shared>,
    rx: mpsc::Receiver<AudioCommand>,
    sample_rate: u32,
    channels: usize,
) {
    thread::spawn(move || {
        ffmpeg_next::init().ok();

        let mut mixer = TimelineMixer::new(sample_rate);
        let mut generation = shared.generation.load(Ordering::Acquire);
        let mut produced_frames: u64 = 0;
        let ring_target = (RING_TARGET_SECONDS * sample_rate as f64) as usize * channels;

        let mut stereo = vec![0.0f32; CHUNK_FRAMES * 2];
        let mut interleaved = vec![0.0f32; CHUNK_FRAMES * channels];

        loop {
            match rx.try_recv() {
                Ok(AudioCommand::SetProgram(segments)) => mixer.set_program(segments),
                Ok(AudioCommand::Shutdown) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => {}
            }

            let current = shared.generation.load(Ordering::Acquire);
            if current != generation {
                generation = current;
                produced_frames = 0;
                if let Ok(mut ring) = shared.ring.lock() {
                    ring.clear();
                }
                mixer.invalidate();
            }

            let ring_len = shared.ring.lock().map(|r| r.len()).unwrap_or(0);
            if ring_len >= ring_target {
                thread::sleep(Duration::from_millis(4));
                continue;
            }

            let chunk_start = shared.origin() + produced_frames as f64 / sample_rate as f64;
            mixer.render(chunk_start, &mut stereo);

            let volume = shared.volume_milli.load(Ordering::Relaxed) as f32 / 1000.0;
            for frame in 0..CHUNK_FRAMES {
                let left = (stereo[frame * 2] * volume).clamp(-1.0, 1.0);
                let right = (stereo[frame * 2 + 1] * volume).clamp(-1.0, 1.0);
                for channel in 0..channels {
                    interleaved[frame * channels + channel] =
                        if channel % 2 == 0 { left } else { right };
                }
            }

            // A seek during rendering makes this chunk stale.
            if shared.generation.load(Ordering::Acquire) != generation {
                continue;
            }

            if let Ok(mut ring) = shared.ring.lock() {
                ring.extend(interleaved.iter().copied());
            }
            produced_frames += CHUNK_FRAMES as u64;
        }
    });
}
