use crate::action_engine::tools::command::{ensure_parent_dir, run_ffmpeg, validate_path};
use crate::action_engine::tools::error::{ActionEngineError, ActionResult};
use std::path::{Path, PathBuf};

/// Removes the audio stream from `input` (video-only output).
pub async fn mute(input: impl AsRef<Path>, output: impl AsRef<Path>) -> ActionResult<PathBuf> {
    let input = input.as_ref();
    let output = output.as_ref();
    validate_path(input, "input")?;
    validate_path(output, "output")?;
    ensure_parent_dir(output).await?;

    run_ffmpeg([
        "-i",
        &path_arg(input),
        "-c:v",
        "copy",
        "-an",
        &path_arg(output),
    ])
    .await?;

    Ok(output.to_path_buf())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioExtractCodec {
    /// Copy the audio bitstream when the container allows it.
    #[default]
    Copy,
    /// Re-encode to AAC.
    Aac,
    /// Re-encode to WAV PCM s16le.
    Wav,
}

/// Extracts the primary audio track from `input` into `output`.
pub async fn extract_audio(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    codec: AudioExtractCodec,
) -> ActionResult<PathBuf> {
    let input = input.as_ref();
    let output = output.as_ref();
    validate_path(input, "input")?;
    validate_path(output, "output")?;
    ensure_parent_dir(output).await?;

    let mut args: Vec<String> = vec!["-i".into(), path_arg(input), "-vn".into()];
    match codec {
        AudioExtractCodec::Copy => {
            args.push("-c:a".into());
            args.push("copy".into());
        }
        AudioExtractCodec::Aac => {
            args.push("-c:a".into());
            args.push("aac".into());
            args.push("-b:a".into());
            args.push("192k".into());
        }
        AudioExtractCodec::Wav => {
            args.push("-c:a".into());
            args.push("pcm_s16le".into());
        }
    }
    args.push(path_arg(output));

    run_ffmpeg(&args).await?;
    Ok(output.to_path_buf())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioReplaceMode {
    /// Drop original audio; use the new track only.
    #[default]
    Replace,
    /// Mix original + new audio.
    Mix,
}

#[derive(Debug, Clone)]
pub struct AudioMixOptions {
    pub mode: AudioReplaceMode,
    /// Weight for the original track when mixing (0.0–1.0+).
    pub original_volume: f64,
    /// Weight for the replacement track when mixing.
    pub new_volume: f64,
    /// Copy video stream without re-encode when possible.
    pub copy_video: bool,
}

impl Default for AudioMixOptions {
    fn default() -> Self {
        Self {
            mode: AudioReplaceMode::Replace,
            original_volume: 1.0,
            new_volume: 1.0,
            copy_video: true,
        }
    }
}

/// Replaces or mixes audio from `audio` into `video`, writing `output`.
pub async fn replace_or_mix_audio(
    video: impl AsRef<Path>,
    audio: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: AudioMixOptions,
) -> ActionResult<PathBuf> {
    let video = video.as_ref();
    let audio = audio.as_ref();
    let output = output.as_ref();
    validate_path(video, "video")?;
    validate_path(audio, "audio")?;
    validate_path(output, "output")?;
    ensure_parent_dir(output).await?;

    if !options.original_volume.is_finite() || options.original_volume < 0.0 {
        return Err(ActionEngineError::invalid(
            "original_volume must be finite and >= 0",
        ));
    }
    if !options.new_volume.is_finite() || options.new_volume < 0.0 {
        return Err(ActionEngineError::invalid(
            "new_volume must be finite and >= 0",
        ));
    }

    let video_s = path_arg(video);
    let audio_s = path_arg(audio);
    let output_s = path_arg(output);
    let vcodec = if options.copy_video { "copy" } else { "libx264" };

    match options.mode {
        AudioReplaceMode::Replace => {
            let mut args = vec![
                "-i".into(),
                video_s,
                "-i".into(),
                audio_s,
                "-map".into(),
                "0:v:0".into(),
                "-map".into(),
                "1:a:0".into(),
                "-c:v".into(),
                vcodec.into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-shortest".into(),
            ];
            if !options.copy_video {
                args.push("-preset".into());
                args.push("veryfast".into());
                args.push("-crf".into());
                args.push("18".into());
            }
            args.push(output_s);
            run_ffmpeg(&args).await?;
        }
        AudioReplaceMode::Mix => {
            let filter = format!(
                "[0:a]volume={orig}[a0];[1:a]volume={new}[a1];\
                 [a0][a1]amix=inputs=2:duration=shortest:dropout_transition=0[aout]",
                orig = options.original_volume,
                new = options.new_volume,
            );
            let mut args = vec![
                "-i".into(),
                video_s,
                "-i".into(),
                audio_s,
                "-filter_complex".into(),
                filter,
                "-map".into(),
                "0:v:0".into(),
                "-map".into(),
                "[aout]".into(),
                "-c:v".into(),
                vcodec.into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
            ];
            if !options.copy_video {
                args.push("-preset".into());
                args.push("veryfast".into());
                args.push("-crf".into());
                args.push("18".into());
            }
            args.push(output_s);
            run_ffmpeg(&args).await?;
        }
    }

    Ok(output.to_path_buf())
}

/// Convenience: replace video audio with a new track.
pub async fn replace_audio(
    video: impl AsRef<Path>,
    audio: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> ActionResult<PathBuf> {
    replace_or_mix_audio(video, audio, output, AudioMixOptions::default()).await
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
