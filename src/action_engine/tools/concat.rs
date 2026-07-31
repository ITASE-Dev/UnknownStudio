use crate::action_engine::tools::command::{ensure_parent_dir, run_ffmpeg, validate_path};
use crate::action_engine::tools::error::{ActionEngineError, ActionResult};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConcatCodecMode {
    /// Demuxer concat + stream copy (requires matching codecs/params).
    #[default]
    Copy,
    /// Re-encode while concatenating (tolerant of mismatched inputs).
    Reencode,
}

#[derive(Debug, Clone)]
pub struct ConcatOptions {
    pub codec: ConcatCodecMode,
}

impl Default for ConcatOptions {
    fn default() -> Self {
        Self {
            codec: ConcatCodecMode::Copy,
        }
    }
}

/// Joins `inputs` in order into a single `output` file.
pub async fn concatenate(
    inputs: &[impl AsRef<Path>],
    output: impl AsRef<Path>,
    options: ConcatOptions,
) -> ActionResult<PathBuf> {
    if inputs.is_empty() {
        return Err(ActionEngineError::invalid(
            "concatenate requires at least one input",
        ));
    }

    let output = output.as_ref();
    validate_path(output, "output")?;
    ensure_parent_dir(output).await?;

    for (i, input) in inputs.iter().enumerate() {
        validate_path(input.as_ref(), &format!("input[{i}]"))?;
    }

    match options.codec {
        ConcatCodecMode::Copy => concat_demuxer_copy(inputs, output).await,
        ConcatCodecMode::Reencode => concat_filter_reencode(inputs, output).await,
    }
}

/// Stream-copy concat via the concat demuxer.
async fn concat_demuxer_copy(
    inputs: &[impl AsRef<Path>],
    output: &Path,
) -> ActionResult<PathBuf> {
    let list_path = temp_list_path(output);
    let list_body = build_concat_list(inputs)?;
    tokio::fs::write(&list_path, list_body).await?;

    let result = run_ffmpeg([
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        &path_arg(&list_path),
        "-c",
        "copy",
        &path_arg(output),
    ])
    .await;

    let _ = tokio::fs::remove_file(&list_path).await;
    result?;
    Ok(output.to_path_buf())
}

/// Filter-graph concat with re-encode — works across mismatched streams.
async fn concat_filter_reencode(
    inputs: &[impl AsRef<Path>],
    output: &Path,
) -> ActionResult<PathBuf> {
    let n = inputs.len();
    let mut args: Vec<String> = Vec::with_capacity(n * 2 + 12);

    for input in inputs {
        args.push("-i".into());
        args.push(path_arg(input.as_ref()));
    }

    // Assume each input has video+audio. Missing audio is the caller's problem
    // for the reencode path; Copy path is preferred for homogeneous timelines.
    let mut filter = String::with_capacity(n * 24 + 32);
    for i in 0..n {
        filter.push_str(&format!("[{i}:v:0][{i}:a:0]"));
    }
    filter.push_str(&format!("concat=n={n}:v=1:a=1[v][a]"));

    args.push("-filter_complex".into());
    args.push(filter);
    args.push("-map".into());
    args.push("[v]".into());
    args.push("-map".into());
    args.push("[a]".into());
    args.push("-c:v".into());
    args.push("libx264".into());
    args.push("-preset".into());
    args.push("veryfast".into());
    args.push("-crf".into());
    args.push("18".into());
    args.push("-c:a".into());
    args.push("aac".into());
    args.push("-b:a".into());
    args.push("192k".into());
    args.push(path_arg(output));

    run_ffmpeg(&args).await?;
    Ok(output.to_path_buf())
}

fn build_concat_list(inputs: &[impl AsRef<Path>]) -> ActionResult<String> {
    let mut body = String::new();
    for input in inputs {
        let path = input.as_ref();
        // Concat demuxer requires single-quoted paths with escaped quotes/backslashes.
        let escaped = escape_concat_path(path);
        body.push_str("file '");
        body.push_str(&escaped);
        body.push_str("'\n");
    }
    Ok(body)
}

fn escape_concat_path(path: &Path) -> String {
    // Prefer absolute paths so the concat demuxer resolves independently of cwd.
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    absolute
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', r"'\''")
}

fn temp_list_path(output: &Path) -> PathBuf {
    let dir = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("us_concat_{}.txt", Uuid::new_v4()))
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Convenience: stream-copy stitch.
pub async fn concatenate_copy(
    inputs: &[impl AsRef<Path>],
    output: impl AsRef<Path>,
) -> ActionResult<PathBuf> {
    concatenate(inputs, output, ConcatOptions::default()).await
}
