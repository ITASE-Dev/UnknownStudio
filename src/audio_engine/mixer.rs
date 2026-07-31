//! Timeline → stereo. Shared by the realtime mixer thread and (later) export,
//! so what you hear is what gets written.

use crate::audio_engine::source::AudioSource;
use std::path::PathBuf;

/// Sources kept open at once.
const MAX_OPEN_SOURCES: usize = 8;

/// One audible piece of the timeline, in seconds.
#[derive(Clone, PartialEq, Debug)]
pub struct AudioSegment {
    pub start: f32,
    pub end: f32,
    pub path: PathBuf,
    /// Offset into the source that `start` maps to.
    pub media_start: f32,
    pub gain: f32,
}

pub struct TimelineMixer {
    sources: Vec<(PathBuf, AudioSource)>,
    program: Vec<AudioSegment>,
    sample_rate: u32,
}

impl TimelineMixer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sources: Vec::new(),
            program: Vec::new(),
            sample_rate,
        }
    }

    pub fn set_program(&mut self, program: Vec<AudioSegment>) {
        self.program = program;
        self.sources
            .retain(|(path, _)| self.program.iter().any(|s| &s.path == path));
    }

    pub fn invalidate(&mut self) {
        for (_, source) in &mut self.sources {
            source.invalidate();
        }
    }

    /// Fills `out` (interleaved stereo) starting at `timeline_second`.
    /// Overlapping segments sum — that is what a mix is.
    pub fn render(&mut self, timeline_second: f64, out: &mut [f32]) {
        out.iter_mut().for_each(|s| *s = 0.0);

        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }

        let rate = self.sample_rate as f64;
        let span = frames as f64 / rate;

        for index in 0..self.program.len() {
            let segment = self.program[index].clone();
            let (seg_start, seg_end) = (segment.start as f64, segment.end as f64);
            if seg_end <= timeline_second || seg_start >= timeline_second + span {
                continue;
            }

            // Sample range this segment occupies inside the buffer.
            let from = (((seg_start - timeline_second).max(0.0)) * rate).round() as usize;
            let to = (((seg_end - timeline_second).min(span)) * rate).round() as usize;
            let (from, to) = (from.min(frames), to.min(frames));
            if to <= from {
                continue;
            }

            let slice_second = timeline_second + from as f64 / rate;
            let media_second = segment.media_start as f64 + (slice_second - seg_start).max(0.0);

            let rate_hz = self.sample_rate;
            let Some(source) = resolve_source(&mut self.sources, &segment.path, rate_hz) else {
                continue;
            };
            source.mix_into(media_second, &mut out[from * 2..to * 2], segment.gain);
        }
    }
}

fn resolve_source<'a>(
    sources: &'a mut Vec<(PathBuf, AudioSource)>,
    path: &PathBuf,
    sample_rate: u32,
) -> Option<&'a mut AudioSource> {
    if let Some(index) = sources.iter().position(|(p, _)| p == path) {
        return sources.get_mut(index).map(|(_, s)| s);
    }
    let source = AudioSource::open(&path.to_string_lossy(), sample_rate)?;
    sources.push((path.clone(), source));
    if sources.len() > MAX_OPEN_SOURCES {
        sources.remove(0);
    }
    sources.last_mut().map(|(_, s)| s)
}
