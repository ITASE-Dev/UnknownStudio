//! Caption track selection, parsing (json3 / WebVTT) and windowing.
//!
//! All functions here are pure: they take text in and give cues out, so the
//! parsing rules can be tested without touching the network.

use crate::ai_tooling::scraping::models::{CaptionTrack, Cue};
use serde_json::Value;
use std::collections::HashMap;

/// Formats in order of preference — json3 carries exact timings, VTT needs
/// stamp parsing, the rest are last resorts.
const PREFERRED_FORMATS: [&str; 4] = ["json3", "vtt", "srv3", "srv1"];

/// Picks an English track, auto-generated captions included.
pub fn select_english_track(tracks: &HashMap<String, Vec<CaptionTrack>>) -> Option<CaptionTrack> {
    let candidates: Vec<&CaptionTrack> = tracks
        .iter()
        .filter(|(lang, _)| lang.to_ascii_lowercase().starts_with("en"))
        .flat_map(|(_, formats)| formats.iter())
        .filter(|track| !track.url.is_empty())
        .collect();

    for preferred in PREFERRED_FORMATS {
        if let Some(track) = candidates
            .iter()
            .find(|track| track.ext.as_deref() == Some(preferred))
        {
            return Some((*track).clone());
        }
    }
    candidates.first().map(|track| (*track).clone())
}

/// Parses whichever format the track advertises.
pub fn parse_cues(raw: &str, ext: Option<&str>) -> Vec<Cue> {
    match ext {
        Some("json3") => parse_json3(raw),
        _ => parse_vtt(raw),
    }
}

/// YouTube's json3 caption format.
pub fn parse_json3(raw: &str) -> Vec<Cue> {
    let Ok(payload) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };

    payload
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter_map(|event| {
                    let segments = event.get("segs")?.as_array()?;
                    let start = event.get("tStartMs")?.as_f64().unwrap_or(0.0) / 1000.0;
                    let duration = event
                        .get("dDurationMs")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                        / 1000.0;

                    let text: String = segments
                        .iter()
                        .filter_map(|segment| segment.get("utf8").and_then(Value::as_str))
                        .collect::<String>()
                        .replace('\n', " ")
                        .trim()
                        .to_string();

                    (!text.is_empty()).then(|| Cue {
                        start,
                        end: start + duration,
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// WebVTT / SRT-style cues.
pub fn parse_vtt(raw: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let (mut start, mut end) = (0.0, 0.0);
    let mut buffer: Vec<&str> = Vec::new();

    let flush = |start: f64, end: f64, buffer: &mut Vec<&str>, cues: &mut Vec<Cue>| {
        if !buffer.is_empty() {
            cues.push(Cue {
                start,
                end,
                text: buffer.join(" "),
            });
            buffer.clear();
        }
    };

    for line in raw.lines() {
        let line = line.trim();

        if let Some((left, right)) = line.split_once("-->") {
            flush(start, end, &mut buffer, &mut cues);
            start = timestamp_seconds(left);
            // The right side may carry cue settings after the stamp.
            end = timestamp_seconds(right.split_whitespace().next().unwrap_or(right));
            continue;
        }

        let header = line.is_empty()
            || line.starts_with("WEBVTT")
            || line.starts_with("Kind:")
            || line.starts_with("Language:")
            || line.chars().all(|c| c.is_ascii_digit());
        if !header {
            buffer.push(line);
        }
    }
    flush(start, end, &mut buffer, &mut cues);

    cues
}

/// `HH:MM:SS.mmm`, `MM:SS.mmm` or `SS.mmm` → seconds.
fn timestamp_seconds(stamp: &str) -> f64 {
    let parts: Vec<f64> = stamp
        .trim()
        .replace(',', ".")
        .split(':')
        .map(|part| part.trim().parse::<f64>().unwrap_or(0.0))
        .collect();

    match parts.as_slice() {
        [hours, minutes, seconds] => hours * 3600.0 + minutes * 60.0 + seconds,
        [minutes, seconds] => minutes * 60.0 + seconds,
        [seconds] => *seconds,
        _ => 0.0,
    }
}

/// Joins every cue overlapping `[start, end]`, de-duplicated — auto-captions
/// repeat each line as the rolling caption grows.
pub fn slice_window(cues: &[Cue], start: f64, end: f64) -> (String, usize) {
    let mut seen: Vec<String> = Vec::new();

    for cue in cues {
        if cue.end < start || cue.start > end {
            continue;
        }
        let normalized = cue.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() || seen.contains(&normalized) {
            continue;
        }
        seen.push(normalized);
    }

    let text = seen.join(" ").trim().to_string();
    let words = text.split_whitespace().count();
    (text, words)
}

/// Words per minute across a window, guarding a zero-length span.
pub fn words_per_minute(word_count: usize, seconds: f64) -> f64 {
    (word_count as f64 / seconds.max(1e-6)) * 60.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(ext: &str) -> CaptionTrack {
        CaptionTrack {
            url: format!("https://example.test/{ext}"),
            ext: Some(ext.to_string()),
        }
    }

    #[test]
    fn english_tracks_win_and_json3_is_preferred() {
        let tracks = HashMap::from([
            ("de".to_string(), vec![track("json3")]),
            ("en-US".to_string(), vec![track("vtt"), track("json3")]),
        ]);
        let chosen = select_english_track(&tracks).expect("english track");
        assert_eq!(chosen.ext.as_deref(), Some("json3"));
        assert!(chosen.url.ends_with("json3"));

        assert!(select_english_track(&HashMap::new()).is_none());
    }

    #[test]
    fn json3_events_become_cues() {
        let raw = r#"{"events":[
            {"tStartMs":1000,"dDurationMs":2000,"segs":[{"utf8":"hello "},{"utf8":"world"}]},
            {"tStartMs":4000,"dDurationMs":1000,"segs":[{"utf8":"\n"}]}
        ]}"#;
        let cues = parse_json3(raw);
        assert_eq!(cues.len(), 1, "blank cues are dropped");
        assert_eq!(cues[0].text, "hello world");
        assert_eq!((cues[0].start, cues[0].end), (1.0, 3.0));
        assert!(parse_json3("not json").is_empty());
    }

    #[test]
    fn vtt_blocks_become_cues() {
        let raw = "WEBVTT\nKind: captions\n\n1\n00:00:01.000 --> 00:00:03.500 align:start\nfirst line\nsecond line\n\n00:01:00.000 --> 00:01:02.000\nlater\n";
        let cues = parse_vtt(raw);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "first line second line");
        assert_eq!((cues[0].start, cues[0].end), (1.0, 3.5));
        assert_eq!(cues[1].start, 60.0);
    }

    #[test]
    fn windowing_drops_the_rolling_duplicates() {
        let cues = vec![
            Cue { start: 0.0, end: 5.0, text: "before".into() },
            Cue { start: 9.0, end: 11.0, text: "the  hook".into() },
            Cue { start: 11.0, end: 13.0, text: "the hook".into() },
            Cue { start: 13.0, end: 15.0, text: "lands here".into() },
            Cue { start: 40.0, end: 42.0, text: "after".into() },
        ];
        let (text, words) = slice_window(&cues, 8.0, 20.0);
        assert_eq!(text, "the hook lands here");
        assert_eq!(words, 4);
    }

    #[test]
    fn wpm_is_per_minute_and_never_divides_by_zero() {
        assert_eq!(words_per_minute(30, 20.0), 90.0);
        assert!(words_per_minute(5, 0.0).is_finite());
    }
}
