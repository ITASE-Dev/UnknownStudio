//! Single-file video decoder: seek, decode-forward, scale to RGB24.
//!
//! Time is always in seconds of *source* media — the timeline's frame rate is
//! none of the decoder's business, so a 24 fps source plays correctly under a
//! 30 fps program.

use ffmpeg::codec::threading::{self, Config as ThreadConfig};
use ffmpeg::format::{input, Pixel};
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as ScalingContext, flag::Flags};
use ffmpeg::util::frame::video::Video;
use ffmpeg::Rational;
use ffmpeg_next as ffmpeg;

/// Forward distance still served by sequential reads instead of a seek. Keep it
/// under a typical keyframe interval or seeking becomes the cheaper path.
const SEQUENTIAL_WINDOW_SECONDS: f64 = 1.0;
const EPS: f64 = 1e-6;
/// Margin taken off a seek target so the keyframe before it is caught.
const SEEK_BACKOFF_SECONDS: f64 = 0.05;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    /// Program monitor.
    Full,
    /// Fast, low-resolution frame while scrubbing.
    Proxy,
    /// Media pool / filmstrip thumbnail.
    Thumb,
}

impl Quality {
    fn max_width(self) -> u32 {
        match self {
            Self::Full => 960,
            Self::Proxy => 480,
            Self::Thumb => 320,
        }
    }

    fn flags(self) -> Flags {
        match self {
            Self::Full => Flags::BICUBIC,
            _ => Flags::BILINEAR,
        }
    }
}

pub struct RgbFrame {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct VideoDecoder {
    ictx: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Video,
    scaler: Option<ScalingContext>,
    scaler_key: Option<(Pixel, u32, u32, Quality)>,
    stream_index: usize,
    time_base: Rational,
    frame_duration: f64,
    duration_seconds: f64,
    /// Last successfully decoded frame, kept for re-scaling.
    last_frame: Video,
    /// Separate `receive_frame` target: the call unrefs its destination, so
    /// decoding straight into `last_frame` would destroy a good frame whenever
    /// a call fails (EAGAIN, end of stream).
    scratch_frame: Video,
    last_pts: Option<f64>,
    eof: bool,
}

impl VideoDecoder {
    pub fn new(path: &str) -> Result<Self, ffmpeg::Error> {
        ffmpeg::init()?;

        let ictx = input(path)?;
        let stream = ictx
            .streams()
            .best(Type::Video)
            .ok_or(ffmpeg::Error::StreamNotFound)?;

        let stream_index = stream.index();
        let time_base = stream.time_base();
        let source_fps = super::probe::detect_fps(&stream);
        let duration_seconds = super::probe::compute_duration_seconds(&ictx, &stream).max(0.001);

        let mut context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
        // count = 0 → FFmpeg uses every core.
        context.set_threading(ThreadConfig {
            kind: threading::Type::Frame,
            count: 0,
            ..Default::default()
        });
        let decoder = context.decoder().video()?;

        Ok(Self {
            ictx,
            decoder,
            scaler: None,
            scaler_key: None,
            stream_index,
            time_base,
            frame_duration: 1.0 / source_fps,
            duration_seconds,
            last_frame: Video::empty(),
            scratch_frame: Video::empty(),
            last_pts: None,
            eof: false,
        })
    }

    pub fn source_size(&self) -> (u32, u32) {
        (self.decoder.width(), self.decoder.height())
    }

    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    /// Frame covering `target_second`, scaled to RGB24 at `quality`.
    /// Consecutive requests inside the same source frame do not re-decode.
    pub fn frame_at(&mut self, target_second: f64, quality: Quality) -> Option<RgbFrame> {
        self.locate(target_second)?;
        self.scale(quality)
    }

    fn locate(&mut self, target_second: f64) -> Option<()> {
        let last_playable = (self.duration_seconds - self.frame_duration * 0.5).max(0.0);
        let target = target_second.clamp(0.0, last_playable);

        if let Some(pts) = self.last_pts {
            // Held frame already covers this instant.
            if target + EPS >= pts && target < pts + self.frame_duration - EPS {
                return Some(());
            }
            // Stream ended: keep showing the last frame rather than black.
            if self.eof && target > pts {
                return Some(());
            }
        }

        let sequential = match self.last_pts {
            Some(pts) => target > pts && target - pts <= SEQUENTIAL_WINDOW_SECONDS,
            None => false,
        };

        // Nothing decoded yet means the stream is already at the start. Seeking
        // there is not just useless: single-image demuxers stop producing
        // packets entirely afterwards.
        let at_start = self.last_pts.is_none() && target <= SEEK_BACKOFF_SECONDS;

        if !sequential && !at_start {
            self.seek_to(target)?;
        }
        self.decode_until(target)
    }

    /// Seek + flush. Without the flush, stale P/B frames mix into the new GOP.
    fn seek_to(&mut self, target: f64) -> Option<()> {
        let backed_off = (target - SEEK_BACKOFF_SECONDS).max(0.0);
        let ts = (backed_off * f64::from(ffmpeg::ffi::AV_TIME_BASE)) as i64;

        if self.ictx.seek(ts, ..ts).is_err() {
            self.ictx.seek(ts, i64::MIN..i64::MAX).ok()?;
        }

        self.decoder.flush();
        self.last_pts = None;
        self.eof = false;
        Some(())
    }

    fn decode_until(&mut self, target: f64) -> Option<()> {
        let mut guard = 0u32;

        loop {
            while let Some(pts) = self.receive_next() {
                self.last_pts = Some(pts);
                if pts + self.frame_duration > target + EPS {
                    return Some(());
                }
            }

            guard += 1;
            if guard > 4096 {
                // Broken stream: show whatever we hold.
                return self.last_pts.map(|_| ());
            }

            if self.feed_packet() {
                continue;
            }

            let _ = self.decoder.send_eof();
            while let Some(pts) = self.receive_next() {
                self.last_pts = Some(pts);
                if pts + self.frame_duration > target + EPS {
                    self.eof = true;
                    return Some(());
                }
            }
            self.eof = true;
            return self.last_pts.map(|_| ());
        }
    }

    fn receive_next(&mut self) -> Option<f64> {
        let time_base = self.time_base;
        let Self {
            decoder,
            last_frame,
            scratch_frame,
            ..
        } = self;

        decoder.receive_frame(scratch_frame).ok()?;
        std::mem::swap(scratch_frame, last_frame);

        let raw = last_frame.pts().or_else(|| last_frame.timestamp()).unwrap_or(0);
        let den = f64::from(time_base.denominator());
        Some(raw as f64 * f64::from(time_base.numerator()) / if den == 0.0 { 1.0 } else { den })
    }

    fn feed_packet(&mut self) -> bool {
        let stream_index = self.stream_index;
        let Self { ictx, decoder, .. } = self;

        for (stream, packet) in ictx.packets() {
            if stream.index() != stream_index {
                continue;
            }
            let _ = decoder.send_packet(&packet);
            return true;
        }
        false
    }

    fn scale(&mut self, quality: Quality) -> Option<RgbFrame> {
        if self.last_pts.is_none() || self.last_frame.width() == 0 {
            return None;
        }

        let (src_format, src_w, src_h) = (
            self.last_frame.format(),
            self.last_frame.width(),
            self.last_frame.height(),
        );
        self.ensure_scaler(quality, src_format, src_w, src_h)?;

        let Self {
            scaler, last_frame, ..
        } = self;
        copy_scaled_rgb(scaler.as_mut()?, last_frame)
    }

    fn ensure_scaler(
        &mut self,
        quality: Quality,
        src_format: Pixel,
        src_w: u32,
        src_h: u32,
    ) -> Option<()> {
        let key = (src_format, src_w, src_h, quality);
        if self.scaler_key == Some(key) && self.scaler.is_some() {
            return Some(());
        }

        let (dst_w, dst_h) = fit_size(src_w, src_h, quality.max_width());
        self.scaler = Some(
            ScalingContext::get(
                src_format,
                src_w,
                src_h,
                Pixel::RGB24,
                dst_w,
                dst_h,
                quality.flags(),
            )
            .ok()?,
        );
        self.scaler_key = Some(key);
        Some(())
    }
}

fn copy_scaled_rgb(scaler: &mut ScalingContext, decoded: &Video) -> Option<RgbFrame> {
    let mut rgb_frame = Video::empty();
    scaler.run(decoded, &mut rgb_frame).ok()?;

    let w = rgb_frame.width() as usize;
    let h = rgb_frame.height() as usize;
    let stride = rgb_frame.stride(0);
    let data = rgb_frame.data(0);

    // Rows are stride-padded; copy the visible span of each.
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let start = y * stride;
        let end = start + w * 3;
        if end > data.len() {
            return None;
        }
        rgb.extend_from_slice(&data[start..end]);
    }

    Some(RgbFrame {
        rgb,
        width: w as u32,
        height: h as u32,
    })
}

/// Even dimensions only — several scalers reject odd sizes.
fn fit_size(w: u32, h: u32, max_w: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (max_w, max_w * 9 / 16);
    }
    if w <= max_w {
        return ((w & !1).max(2), (h & !1).max(2));
    }
    let scale = f64::from(max_w) / f64::from(w);
    (
        (max_w & !1).max(2),
        (((f64::from(h) * scale) as u32) & !1).max(2),
    )
}
