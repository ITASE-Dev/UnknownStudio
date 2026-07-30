//! Container probing. Reads headers only — no decoding — so importing a
//! handful of files stays inside a frame budget.

use ffmpeg_next as ffmpeg;
use ffmpeg::media::Type;

pub struct MediaInfo {
    pub duration_seconds: f64,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub has_video: bool,
    pub has_audio: bool,
}

pub fn probe_media(path: &str) -> Result<MediaInfo, ffmpeg::Error> {
    ffmpeg::init()?;
    let ictx = ffmpeg::format::input(path)?;

    let video = ictx.streams().best(Type::Video);
    let audio = ictx.streams().best(Type::Audio);
    let reference = video.as_ref().or(audio.as_ref()).ok_or(ffmpeg::Error::StreamNotFound)?;

    let (mut width, mut height, mut fps) = (0, 0, 30.0);
    if let Some(stream) = video.as_ref() {
        fps = detect_fps(stream);
        if let Ok(context) = ffmpeg::codec::context::Context::from_parameters(stream.parameters()) {
            if let Ok(decoder) = context.decoder().video() {
                width = decoder.width();
                height = decoder.height();
            }
        }
    }

    let duration_seconds = compute_duration_seconds(&ictx, reference).max(0.1);

    Ok(MediaInfo {
        duration_seconds,
        fps,
        width,
        height,
        has_video: video.is_some(),
        has_audio: audio.is_some(),
    })
}

pub(crate) fn detect_fps(stream: &ffmpeg::format::stream::Stream) -> f64 {
    for rate in [stream.avg_frame_rate(), stream.rate()] {
        if rate.numerator() > 0 && rate.denominator() > 0 {
            let fps = f64::from(rate.numerator()) / f64::from(rate.denominator());
            if (1.0..=480.0).contains(&fps) {
                return fps;
            }
        }
    }
    30.0
}

/// Stream duration first, then the container, then frame count — containers
/// disagree about which of the three they populate.
pub(crate) fn compute_duration_seconds(
    ictx: &ffmpeg::format::context::Input,
    stream: &ffmpeg::format::stream::Stream,
) -> f64 {
    let time_base = stream.time_base();
    let stream_dur = stream.duration();
    if stream_dur > 0 && time_base.numerator() > 0 && time_base.denominator() > 0 {
        return stream_dur as f64 * f64::from(time_base.numerator())
            / f64::from(time_base.denominator());
    }

    let container_dur = ictx.duration();
    if container_dur > 0 {
        return container_dur as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);
    }

    let frames = stream.frames();
    let avg = stream.avg_frame_rate();
    if frames > 0 && avg.numerator() > 0 && avg.denominator() > 0 {
        return frames as f64 * f64::from(avg.denominator()) / f64::from(avg.numerator());
    }

    0.0
}
