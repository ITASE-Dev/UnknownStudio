//! Background decode service.
//!
//! Two worker threads keep FFmpeg off the UI thread: one serves the program
//! monitor (always the newest request — stale frames are worthless), one fills
//! media-pool thumbnails. The UI never blocks; it polls.

use crate::media::decoder::{Quality, RgbFrame, VideoDecoder};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

/// Decoders kept open so segment changes don't reopen files.
const MAX_OPEN_DECODERS: usize = 4;

/// One flattened piece of the program: `[start, end)` of timeline seconds
/// showing `path` from `media_start`.
#[derive(Clone, PartialEq, Debug)]
pub struct Segment {
    pub start: f32,
    pub end: f32,
    pub path: PathBuf,
    pub media_start: f32,
}

impl Segment {
    fn contains(&self, seconds: f32) -> bool {
        seconds >= self.start && seconds < self.end
    }

    fn media_seconds(&self, seconds: f32) -> f64 {
        (self.media_start + (seconds - self.start)).max(0.0) as f64
    }
}

enum FrameRequest {
    Program(Vec<Segment>),
    Frame { seconds: f32, quality: Quality },
}

pub enum Decoded {
    /// Program monitor frame at `seconds` of the timeline.
    Frame { seconds: f32, frame: RgbFrame },
    /// Pool thumbnail for `path`.
    Thumbnail { path: PathBuf, frame: RgbFrame },
    /// Nothing to show at this instant (gap or unreadable source).
    Blank { seconds: f32 },
}

pub struct PreviewEngine {
    frames_tx: Sender<FrameRequest>,
    thumbs_tx: Sender<PathBuf>,
    decoded_rx: Receiver<Decoded>,
    /// Last program handed to the worker; resending an identical one would
    /// reset its decoder pool for nothing.
    program: Vec<Segment>,
}

impl Default for PreviewEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewEngine {
    pub fn new() -> Self {
        let (frames_tx, frames_rx) = mpsc::channel::<FrameRequest>();
        let (thumbs_tx, thumbs_rx) = mpsc::channel::<PathBuf>();
        let (decoded_tx, decoded_rx) = mpsc::channel::<Decoded>();

        spawn_frame_thread(frames_rx, decoded_tx.clone());
        spawn_thumb_thread(thumbs_rx, decoded_tx);

        Self {
            frames_tx,
            thumbs_tx,
            decoded_rx,
            program: Vec::new(),
        }
    }

    pub fn program(&self) -> &Vec<Segment> {
        &self.program
    }

    /// Publishes the flattened program if it changed.
    pub fn set_program(&mut self, program: Vec<Segment>) {
        if program == self.program {
            return;
        }
        self.program = program.clone();
        let _ = self.frames_tx.send(FrameRequest::Program(program));
    }

    pub fn request_frame(&self, seconds: f32, quality: Quality) {
        let _ = self.frames_tx.send(FrameRequest::Frame { seconds, quality });
    }

    pub fn request_thumbnail(&self, path: PathBuf) {
        let _ = self.thumbs_tx.send(path);
    }

    /// Non-blocking drain of finished work.
    pub fn poll(&self) -> impl Iterator<Item = Decoded> + '_ {
        std::iter::from_fn(move || self.decoded_rx.try_recv().ok())
    }
}

/// Small LRU of open decoders, keyed by path.
struct DecoderPool {
    entries: Vec<(PathBuf, VideoDecoder)>,
}

impl DecoderPool {
    fn get(&mut self, path: &PathBuf) -> Option<&mut VideoDecoder> {
        if let Some(index) = self.entries.iter().position(|(p, _)| p == path) {
            let entry = self.entries.remove(index);
            self.entries.insert(0, entry);
        } else {
            let decoder = VideoDecoder::new(&path.to_string_lossy()).ok()?;
            self.entries.insert(0, (path.clone(), decoder));
            self.entries.truncate(MAX_OPEN_DECODERS);
        }
        self.entries.first_mut().map(|(_, decoder)| decoder)
    }

    fn retain(&mut self, program: &[Segment]) {
        self.entries
            .retain(|(path, _)| program.iter().any(|s| &s.path == path));
    }
}

fn spawn_frame_thread(requests: Receiver<FrameRequest>, out: Sender<Decoded>) {
    thread::spawn(move || {
        init_ffmpeg();
        let mut pool = DecoderPool {
            entries: Vec::new(),
        };
        let mut program: Vec<Segment> = Vec::new();

        while let Ok(first) = requests.recv() {
            // Collapse the backlog: only the newest program and newest frame
            // request matter, everything in between is already outdated.
            let mut pending: Option<(f32, Quality)> = None;
            let mut absorb = |request| match request {
                FrameRequest::Program(segments) => {
                    program = segments;
                    pool.retain(&program);
                }
                FrameRequest::Frame { seconds, quality } => pending = Some((seconds, quality)),
            };
            absorb(first);
            loop {
                match requests.try_recv() {
                    Ok(next) => absorb(next),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            let Some((seconds, quality)) = pending else {
                continue;
            };

            let decoded = program
                .iter()
                .find(|segment| segment.contains(seconds))
                .cloned()
                .and_then(|segment| {
                    let media_seconds = segment.media_seconds(seconds);
                    pool.get(&segment.path)
                        .and_then(|decoder| decoder.frame_at(media_seconds, quality))
                });

            let message = match decoded {
                Some(frame) => Decoded::Frame { seconds, frame },
                None => Decoded::Blank { seconds },
            };
            if out.send(message).is_err() {
                return;
            }
        }
    });
}

fn spawn_thumb_thread(requests: Receiver<PathBuf>, out: Sender<Decoded>) {
    thread::spawn(move || {
        init_ffmpeg();

        while let Ok(path) = requests.recv() {
            let Ok(mut decoder) = VideoDecoder::new(&path.to_string_lossy()) else {
                continue;
            };
            // A frame slightly inside the file: many clips open on black.
            let second = (decoder.duration_seconds() * 0.1).min(1.0);
            let Some(frame) = decoder
                .frame_at(second, Quality::Thumb)
                .or_else(|| decoder.frame_at(0.0, Quality::Thumb))
            else {
                continue;
            };
            if out.send(Decoded::Thumbnail { path, frame }).is_err() {
                return;
            }
        }
    });
}

fn init_ffmpeg() {
    ffmpeg_next::init().ok();
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
}
