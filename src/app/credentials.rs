//! API keys and service addresses, read from and written back to `.env`.
//!
//! Secrets live in one file the app already reads at startup. Saving merges
//! into it rather than rewriting it, so comments and unrelated entries survive.

use crate::ai_tooling::config::{load_dotenv, AiToolingConfig, ProviderKind, Tuning};
use std::env;
use std::path::PathBuf;

/// Fields the settings form edits, in the order it shows them.
pub const FIELDS: [&str; 6] = [
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "LLM_PROVIDER",
    "OPENAI_MODEL",
    "COMFYUI_URL",
    "YOUTUBE_API_KEY",
];

#[derive(Debug, Clone, Default)]
pub struct Credentials {
    pub openai_key: String,
    pub anthropic_key: String,
    pub provider: String,
    pub openai_model: String,
    pub comfyui_url: String,
    pub youtube_key: String,
}

/// `value`, or `fallback` when it is blank.
fn non_empty(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

impl Credentials {
    /// Current values, from `.env` and the environment.
    pub fn load() -> Self {
        load_dotenv();
        let read = |key: &str| env::var(key).unwrap_or_default().trim().to_string();

        Self {
            openai_key: read("OPENAI_API_KEY"),
            anthropic_key: read("ANTHROPIC_API_KEY"),
            provider: non_empty(read("LLM_PROVIDER"), "openai"),
            openai_model: non_empty(read("OPENAI_MODEL"), "gpt-4o"),
            comfyui_url: read("COMFYUI_URL"),
            youtube_key: read("YOUTUBE_API_KEY"),
        }
    }

    /// Key/value pairs to persist, trimmed.
    pub fn entries(&self) -> Vec<(&'static str, String)> {
        vec![
            ("OPENAI_API_KEY", self.openai_key.trim().to_string()),
            ("ANTHROPIC_API_KEY", self.anthropic_key.trim().to_string()),
            ("LLM_PROVIDER", self.provider.trim().to_string()),
            ("OPENAI_MODEL", self.openai_model.trim().to_string()),
            ("COMFYUI_URL", self.comfyui_url.trim().to_string()),
            ("YOUTUBE_API_KEY", self.youtube_key.trim().to_string()),
        ]
    }

    /// Whether the assistant has what it needs to run.
    pub fn assistant_ready(&self) -> bool {
        match self.provider.trim() {
            "anthropic" => !self.anthropic_key.trim().is_empty(),
            _ => !self.openai_key.trim().is_empty(),
        }
    }

    /// Config for the chat client, built from these values directly so a save
    /// takes effect without restarting or mutating the process environment.
    pub fn to_config(&self) -> AiToolingConfig {
        let optional = |value: &str| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        };

        AiToolingConfig {
            youtube_api_key: self.youtube_key.trim().to_string(),
            channel_ids: Vec::new(),
            provider: match self.provider.trim() {
                "anthropic" => ProviderKind::Anthropic,
                _ => ProviderKind::OpenAi,
            },
            anthropic_api_key: optional(&self.anthropic_key),
            anthropic_model: env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-opus-5".to_string()),
            openai_api_key: optional(&self.openai_key),
            openai_model: match self.openai_model.trim() {
                "" => "gpt-4o".to_string(),
                model => model.to_string(),
            },
            openai_base_url: env::var("OPENAI_BASE_URL")
                .ok()
                .map(|url| url.trim().to_string())
                .filter(|url| !url.is_empty())
                .unwrap_or_else(|| {
                    crate::ai_tooling::config::OPENAI_DEFAULT_BASE_URL.to_string()
                }),
            tuning: Tuning::default(),
        }
    }

    /// Writes the values into `.env`, creating it if needed. Returns the file.
    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = env_path();
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let merged = merge_env(&existing, &self.entries());

        std::fs::write(&path, merged)?;
        Ok(path)
    }
}

/// The `.env` the loader would find, or the working directory's.
pub fn env_path() -> PathBuf {
    load_dotenv().unwrap_or_else(|| {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".env")
    })
}

/// Updates `key=value` lines in place, appends the rest, and leaves everything
/// else — comments, blank lines, unrelated keys — exactly as it was.
///
/// An empty value removes the line rather than writing `KEY=`: a blank entry is
/// picked up as a set-but-empty variable, which reads as "configured" and then
/// fails at the first request.
pub fn merge_env(existing: &str, updates: &[(&str, String)]) -> String {
    let mut written: Vec<&str> = Vec::new();
    let mut out: Vec<String> = Vec::new();

    for line in existing.lines() {
        let Some(key) = line_key(line) else {
            out.push(line.to_string());
            continue;
        };

        match updates.iter().find(|(name, _)| *name == key) {
            Some((name, value)) => {
                written.push(name);
                if !value.is_empty() {
                    out.push(format!("{name}={}", quoted(value)));
                }
            }
            None => out.push(line.to_string()),
        }
    }

    let missing: Vec<&(&str, String)> = updates
        .iter()
        .filter(|(name, value)| !value.is_empty() && !written.contains(name))
        .collect();

    if !missing.is_empty() {
        if out.last().is_some_and(|line| !line.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("# Added by Unknown Studio settings".to_string());
        for (name, value) in missing {
            out.push(format!("{name}={}", quoted(value)));
        }
    }

    let mut text = out.join("\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// `KEY` of an assignment line, ignoring comments and blanks.
fn line_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let key = trimmed.split('=').next()?.trim();
    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then_some(key)
        .filter(|key| !key.is_empty())
}

/// Quotes values the parser would otherwise choke on — the lesson of a raw ODBC
/// connection string taking the whole file down with it.
fn quoted(value: &str) -> String {
    let needs_quotes = value
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, ';' | '{' | '}' | '#' | '\'' | '"'));

    if needs_quotes {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// Shows enough of a secret to recognise it, never enough to use it.
pub fn mask(secret: &str) -> String {
    let secret = secret.trim();
    match secret.chars().count() {
        0 => "not set".to_string(),
        1..=10 => "•".repeat(secret.chars().count()),
        length => {
            let tail: String = secret.chars().skip(length - 4).collect();
            format!("{}••••{tail}", &secret[..3.min(secret.len())])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn updates(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs
            .iter()
            .map(|(key, value)| (*key, value.to_string()))
            .collect()
    }

    #[test]
    fn an_existing_key_is_replaced_in_place() {
        let existing = "# keys\nOPENAI_API_KEY=old\nOPENAI_MODEL=gpt-4o\n";
        let merged = merge_env(&existing, &updates(&[("OPENAI_API_KEY", "sk-new")]));

        assert_eq!(merged, "# keys\nOPENAI_API_KEY=sk-new\nOPENAI_MODEL=gpt-4o\n");
    }

    #[test]
    fn unrelated_lines_and_comments_survive_untouched() {
        let existing = "# --- MS SQL ---\nDB_CONN_STR=\"Driver={ODBC};Server=a\"\n\n# LLM\nLLM_PROVIDER=openai\n";
        let merged = merge_env(&existing, &updates(&[("LLM_PROVIDER", "anthropic")]));

        assert!(merged.contains("# --- MS SQL ---"));
        assert!(merged.contains("DB_CONN_STR=\"Driver={ODBC};Server=a\""));
        assert!(merged.contains("LLM_PROVIDER=anthropic"));
        assert!(!merged.contains("LLM_PROVIDER=openai"));
    }

    #[test]
    fn a_new_key_is_appended_under_its_own_heading() {
        let merged = merge_env("EXISTING=1\n", &updates(&[("COMFYUI_URL", "http://x:8188")]));

        assert!(merged.starts_with("EXISTING=1\n"));
        assert!(merged.contains("# Added by Unknown Studio settings"));
        assert!(merged.contains("COMFYUI_URL=http://x:8188"));
    }

    #[test]
    fn clearing_a_field_removes_the_line_rather_than_blanking_it() {
        // `KEY=` reads as set-but-empty and fails later; absence is honest.
        let merged = merge_env("OPENAI_API_KEY=sk-old\nKEEP=1\n", &updates(&[("OPENAI_API_KEY", "")]));

        assert!(!merged.contains("OPENAI_API_KEY"));
        assert!(merged.contains("KEEP=1"));
    }

    #[test]
    fn awkward_values_are_quoted_so_the_file_still_parses() {
        let merged = merge_env(
            "",
            &updates(&[("DB", "Driver={ODBC Driver 17};Server=HOST")]),
        );
        assert!(merged.contains("DB=\"Driver={ODBC Driver 17};Server=HOST\""));

        // A plain key needs no quotes.
        let plain = merge_env("", &updates(&[("OPENAI_API_KEY", "sk-proj-abc123")]));
        assert!(plain.contains("OPENAI_API_KEY=sk-proj-abc123"));
    }

    #[test]
    fn writing_an_empty_file_still_ends_with_a_newline() {
        let merged = merge_env("", &updates(&[("A", "1")]));
        assert!(merged.ends_with('\n'));
    }

    #[test]
    fn masking_shows_the_shape_not_the_secret() {
        assert_eq!(mask(""), "not set");
        assert_eq!(mask("sk-proj-1234567890abcd"), "sk-••••abcd");
        assert_eq!(mask("short"), "•••••");
        assert!(!mask("sk-proj-1234567890abcd").contains("1234567890"));
    }

    #[test]
    fn readiness_follows_the_selected_provider() {
        let mut credentials = Credentials {
            provider: "openai".into(),
            openai_key: "sk-x".into(),
            ..Credentials::default()
        };
        assert!(credentials.assistant_ready());

        // Switching provider without its key is not ready.
        credentials.provider = "anthropic".into();
        assert!(!credentials.assistant_ready());

        credentials.anthropic_key = "sk-ant".into();
        assert!(credentials.assistant_ready());
    }

    #[test]
    fn the_config_it_builds_matches_the_selected_provider() {
        let credentials = Credentials {
            provider: "anthropic".into(),
            anthropic_key: "sk-ant".into(),
            openai_key: "sk-openai".into(),
            openai_model: "gpt-4o-mini".into(),
            ..Credentials::default()
        };

        let config = credentials.to_config();
        assert_eq!(config.provider, ProviderKind::Anthropic);
        assert_eq!(config.anthropic_api_key.as_deref(), Some("sk-ant"));
        assert_eq!(config.openai_model, "gpt-4o-mini");
        assert!(config.openai_base_url.starts_with("http"));
    }
}
