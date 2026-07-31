use std::fmt;
use std::io;
use std::process::ExitStatus;

/// Failures raised by the FFmpeg-backed editing toolkit.
#[derive(Debug)]
pub enum ActionEngineError {
    Io(io::Error),
    InvalidArgument(String),
    FfmpegFailed {
        status: Option<i32>,
        message: String,
    },
    MissingBinary(String),
}

impl ActionEngineError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidArgument(msg.into())
    }

    pub fn from_status(status: ExitStatus) -> Self {
        Self::FfmpegFailed {
            status: status.code(),
            message: format!("ffmpeg exited with {status}"),
        }
    }
}

impl fmt::Display for ActionEngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            Self::FfmpegFailed { status, message } => match status {
                Some(code) => write!(f, "ffmpeg failed (exit {code}): {message}"),
                None => write!(f, "ffmpeg failed: {message}"),
            },
            Self::MissingBinary(name) => write!(f, "required binary not found: {name}"),
        }
    }
}

impl std::error::Error for ActionEngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for ActionEngineError {
    fn from(err: io::Error) -> Self {
        if err.kind() == io::ErrorKind::NotFound {
            Self::MissingBinary("ffmpeg".into())
        } else {
            Self::Io(err)
        }
    }
}

pub type ActionResult<T> = Result<T, ActionEngineError>;
