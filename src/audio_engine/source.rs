//! One file's audio stream: decode + resample to stereo f32 at the device rate,
//! with an internal buffer so sequential reads don't re-seek.

use ffmpeg::format::{input, sample::Type as SampleType, Sample};
use ffmpeg::media::Type as MediaType;
use ffmpeg::util::channel_layout::ChannelLayout;
use ffmpeg_next as ffmpeg;
use std::collections::VecDeque;

pub struct AudioSource {
    ictx: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Audio,
    resampler: Option<ffmpeg::software::resampling::Context>,
    stream_index: usize,
    time_base: f64,
    target_rate: u32,
    /// Interleaved stereo f32.
    buffer: VecDeque<f32>,
    /// Media second of the buffer's first frame.
    buffer_start: Option<f64>,
    eof: bool,
}

impl AudioSource {
    pub fn open(path: &str, target_rate: u32) -> Option<Self> {
        let ictx = input(path).ok()?;
        let stream = ictx.streams().best(MediaType::Audio)?;
        let stream_index = stream.index();
        let time_base = f64::from(stream.time_base());

        let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters()).ok()?;
        let decoder = context.decoder().audio().ok()?;

        Some(Self {
            ictx,
            decoder,
            resampler: None,
            stream_index,
            time_base,
            target_rate,
            buffer: VecDeque::new(),
            buffer_start: None,
            eof: false,
        })
    }

    /// Drops buffered audio after a seek.
    pub fn invalidate(&mut self) {
        self.buffer.clear();
        self.buffer_start = None;
    }

    fn seek(&mut self, second: f64) {
        let target = (second / self.time_base.max(f64::MIN_POSITIVE)) as i64;
        let backstop = target
            .saturating_sub((0.5 / self.time_base.max(1e-9)) as i64)
            .max(0);
        let _ = self.ictx.seek(target, backstop..target.saturating_add(1));
        self.decoder.flush();
        self.buffer.clear();
        self.buffer_start = None;
        self.eof = false;
    }

    /// Adds `out.len() / 2` stereo frames starting at `media_second`, scaled by
    /// `gain`. Additive: several sources mix into the same buffer.
    pub fn mix_into(&mut self, media_second: f64, out: &mut [f32], gain: f32) {
        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }

        let buffered_seconds = self.buffer.len() as f64 / 2.0 / self.target_rate as f64;
        let needs_seek = match self.buffer_start {
            None => true,
            // Backwards, or so far ahead that decoding there is cheaper.
            Some(start) => {
                media_second < start - 0.05 || media_second > start + buffered_seconds + 1.0
            }
        };
        if needs_seek {
            self.seek(media_second);
        }

        // Drop samples that sit before the requested instant.
        if let Some(start) = self.buffer_start {
            if media_second > start {
                let drop_frames = ((media_second - start) * self.target_rate as f64).round() as usize;
                let drop_samples = (drop_frames * 2).min(self.buffer.len());
                self.buffer.drain(..drop_samples);
                self.buffer_start =
                    Some(start + drop_samples as f64 / 2.0 / self.target_rate as f64);
            }
        }

        while self.buffer.len() < frames * 2 && !self.eof {
            if !self.decode_next() {
                break;
            }
        }

        let available = (self.buffer.len() / 2).min(frames);
        for i in 0..available * 2 {
            out[i] += self.buffer[i] * gain;
        }

        self.buffer.drain(..available * 2);
        if let Some(start) = self.buffer_start {
            self.buffer_start = Some(start + available as f64 / self.target_rate as f64);
        }
    }

    fn next_audio_packet(&mut self) -> Option<ffmpeg::codec::packet::Packet> {
        let wanted = self.stream_index;
        for (stream, packet) in self.ictx.packets() {
            if stream.index() == wanted {
                return Some(packet);
            }
        }
        None
    }

    /// Reads and decodes one packet. `false` once the file is exhausted.
    fn decode_next(&mut self) -> bool {
        let Some(packet) = self.next_audio_packet() else {
            self.eof = true;
            let _ = self.decoder.send_eof();
            self.drain_decoder(None);
            return false;
        };

        let pts = packet.pts().or(packet.dts());
        if self.decoder.send_packet(&packet).is_err() {
            return true;
        }
        self.drain_decoder(pts);
        true
    }

    fn drain_decoder(&mut self, fallback_pts: Option<i64>) {
        let mut decoded = ffmpeg::frame::Audio::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            if self.buffer_start.is_none() {
                let stamp = decoded.pts().or(fallback_pts).unwrap_or(0);
                self.buffer_start = Some(stamp as f64 * self.time_base);
            }
            self.append(&decoded);
        }
    }

    fn append(&mut self, frame: &ffmpeg::frame::Audio) {
        if frame.samples() == 0 {
            return;
        }

        if self.resampler.is_none() {
            self.resampler = ffmpeg::software::resampling::Context::get(
                frame.format(),
                frame.channel_layout(),
                frame.rate(),
                Sample::F32(SampleType::Packed),
                ChannelLayout::default(2),
                self.target_rate,
            )
            .ok();
        }

        let Some(resampler) = self.resampler.as_mut() else {
            return;
        };

        let mut output = ffmpeg::frame::Audio::empty();
        if resampler.run(frame, &mut output).is_err() {
            return;
        }
        push_packed_f32(&mut self.buffer, &output);

        // Flush resampler latency, otherwise drift accumulates over time.
        let mut guard = 0;
        while resampler.delay().is_some() && guard < 8 {
            let mut extra = ffmpeg::frame::Audio::empty();
            if resampler.flush(&mut extra).is_err() || extra.samples() == 0 {
                break;
            }
            push_packed_f32(&mut self.buffer, &extra);
            guard += 1;
        }
    }
}

fn push_packed_f32(buffer: &mut VecDeque<f32>, frame: &ffmpeg::frame::Audio) {
    let count = frame.samples() * 2;
    if count == 0 {
        return;
    }
    let bytes = frame.data(0);
    let usable = count.min(bytes.len() / 4);
    buffer.extend(
        bytes[..usable * 4]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
    );
}
