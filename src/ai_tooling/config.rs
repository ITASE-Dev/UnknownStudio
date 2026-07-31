//! `.env`-only configuration. Secrets are read from the environment and never
//! written back into any struct that gets serialized.

use crate::ai_tooling::{AiToolingError, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;

pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-5";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";

/// Directories searched upward for `.env`, from the working directory and from
/// the executable's own location — a GUI launched by double-click rarely starts
/// in the project folder.
const MAX_SEARCH_DEPTH: usize = 6;

static DOTENV: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Loads `.env` into the process environment, once, without overriding
/// variables the environment already sets. Returns the file that was used.
///
/// Call this before anything reads a key — `main` does, so subsystems that use
/// plain `std::env::var` see the same values.
pub fn load_dotenv() -> Option<PathBuf> {
    DOTENV
        .get_or_init(|| {
            let path = locate_dotenv()?;
            let file = std::fs::File::open(&path).ok()?;
            for (key, value) in parse_pairs(file) {
                if env::var_os(&key).is_none() {
                    // SAFETY: called once at startup, before any thread that
                    // reads the environment has been spawned.
                    unsafe { env::set_var(&key, &value) };
                }
            }
            Some(path)
        })
        .clone()
}

/// Key/value pairs from a `.env` stream.
///
/// Malformed lines are skipped rather than aborting the file: one unquoted
/// value (an ODBC connection string, say) must not cost every variable after
/// it — which is exactly how a missing API key can look like a missing key.
fn parse_pairs(reader: impl Read) -> Vec<(String, String)> {
    dotenvy::Iter::new(reader).flatten().collect()
}

fn locate_dotenv() -> Option<PathBuf> {
    let roots = [
        env::current_dir().ok(),
        env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)),
    ];

    roots
        .into_iter()
        .flatten()
        .find_map(|root| {
            root.ancestors()
                .take(MAX_SEARCH_DEPTH)
                .map(|dir| dir.join(".env"))
                .find(|candidate| candidate.is_file())
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }
}

impl FromStr for ProviderKind {
    type Err = AiToolingError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            other => Err(AiToolingError::UnsupportedProvider(other.to_string())),
        }
    }
}

/// Tuning knobs for the outlier math and the transcript window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tuning {
    /// Uploads sampled per channel. Below ~20 the trimmed mean is meaningless.
    pub max_videos: usize,
    /// Fraction dropped from each end before averaging.
    pub trim_percentile: f64,
    /// A video is an outlier above `average * multiplier`.
    pub outlier_multiplier: f64,
    /// Half-width of the transcript window around the replay peak.
    pub peak_window_seconds: f64,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            max_videos: 100,
            trim_percentile: 0.05,
            outlier_multiplier: 3.0,
            peak_window_seconds: 10.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiToolingConfig {
    pub youtube_api_key: String,
    pub channel_ids: Vec<String>,
    pub provider: ProviderKind,
    pub anthropic_api_key: Option<String>,
    pub anthropic_model: String,
    pub openai_api_key: Option<String>,
    pub openai_model: String,
    pub openai_base_url: String,
    pub tuning: Tuning,
}

impl AiToolingConfig {
    /// Reads `.env` (without overriding real environment variables) and
    /// validates everything the selected provider needs.
    pub fn load() -> Result<Self> {
        load_dotenv();

        let provider = optional("LLM_PROVIDER")
            .map(|value| value.parse::<ProviderKind>())
            .transpose()?
            .unwrap_or(ProviderKind::OpenAi);

        let (anthropic_api_key, openai_api_key) = match provider {
            ProviderKind::Anthropic => (Some(required("ANTHROPIC_API_KEY")?), optional("OPENAI_API_KEY")),
            ProviderKind::OpenAi => (optional("ANTHROPIC_API_KEY"), Some(required("OPENAI_API_KEY")?)),
        };

        let channel_ids: Vec<String> = optional("TARGET_CHANNEL_IDS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();

        Ok(Self {
            youtube_api_key: required("YOUTUBE_API_KEY")?,
            channel_ids,
            provider,
            anthropic_api_key,
            anthropic_model: optional("ANTHROPIC_MODEL")
                .unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.to_string()),
            openai_api_key,
            openai_model: optional("OPENAI_MODEL").unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_string()),
            // Always explicit: a blank `OPENAI_BASE_URL=` in .env is picked up as
            // an empty base URL and every request then fails.
            openai_base_url: optional("OPENAI_BASE_URL")
                .unwrap_or_else(|| OPENAI_DEFAULT_BASE_URL.to_string()),
            tuning: Tuning {
                max_videos: number("MAX_VIDEOS", 100)?,
                trim_percentile: number("TRIM_PERCENTILE", 0.05)?,
                outlier_multiplier: number("OUTLIER_MULTIPLIER", 3.0)?,
                peak_window_seconds: number("PEAK_WINDOW_SECONDS", 10.0)?,
            },
        })
    }

    /// Model id of the selected provider, recorded alongside each blueprint.
    pub fn model_id(&self) -> &str {
        match self.provider {
            ProviderKind::Anthropic => &self.anthropic_model,
            ProviderKind::OpenAi => &self.openai_model,
        }
    }

    pub(crate) fn provider_key(&self) -> Result<&str> {
        match self.provider {
            ProviderKind::Anthropic => self
                .anthropic_api_key
                .as_deref()
                .ok_or(AiToolingError::MissingEnv("ANTHROPIC_API_KEY")),
            ProviderKind::OpenAi => self
                .openai_api_key
                .as_deref()
                .ok_or(AiToolingError::MissingEnv("OPENAI_API_KEY")),
        }
    }
}

fn optional(key: &'static str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn required(key: &'static str) -> Result<String> {
    optional(key).ok_or(AiToolingError::MissingEnv(key))
}

fn number<T: FromStr>(key: &'static str, default: T) -> Result<T> {
    match optional(key) {
        None => Ok(default),
        Some(value) => value.parse::<T>().map_err(|_| AiToolingError::InvalidEnv {
            key,
            reason: format!("'{value}' is not a number"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_line_does_not_swallow_the_rest_of_the_file() {
        // The ODBC string is unquoted and full of `{};` — dotenvy rejects the
        // line, and used to abandon the whole file with it.
        let file = concat!(
            r"DB_CONN_STR=Driver={ODBC Driver 17 for SQL Server};Server=HOST\SQLEXPRESS",
            "
LLM_PROVIDER=openai
OPENAI_API_KEY=sk-test-key
"
        );

        let pairs = parse_pairs(file.as_bytes());
        let keys: Vec<&str> = pairs.iter().map(|(key, _)| key.as_str()).collect();

        assert!(
            keys.contains(&"OPENAI_API_KEY"),
            "keys after the bad line survive"
        );
        assert!(keys.contains(&"LLM_PROVIDER"));
        assert_eq!(
            pairs
                .iter()
                .find(|(key, _)| key == "OPENAI_API_KEY")
                .map(|(_, value)| value.as_str()),
            Some("sk-test-key")
        );
    }

    #[test]
    fn quoting_rescues_a_connection_string() {
        let pairs = parse_pairs(
            "DB_CONN_STR=\"Driver={ODBC Driver 17};Server=HOST;Encrypt=yes\"
".as_bytes(),
        );
        assert_eq!(
            pairs.first().map(|(_, value)| value.as_str()),
            Some("Driver={ODBC Driver 17};Server=HOST;Encrypt=yes")
        );
    }

    #[test]
    fn provider_names_round_trip() {
        assert_eq!("anthropic".parse::<ProviderKind>().unwrap(), ProviderKind::Anthropic);
        assert_eq!(" OpenAI ".parse::<ProviderKind>().unwrap(), ProviderKind::OpenAi);
        assert_eq!(ProviderKind::OpenAi.as_str(), "openai");
        assert!(matches!(
            "gemini".parse::<ProviderKind>(),
            Err(AiToolingError::UnsupportedProvider(_))
        ));
    }
}
