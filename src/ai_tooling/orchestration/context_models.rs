//! Lightweight views of editor state, shaped for an LLM rather than for disk.
//!
//! Nothing here carries pixels, samples or file contents — only the metadata an
//! instruction needs to refer to: ids, names, times and track positions.

use serde::{Deserialize, Serialize};
use std::fmt::Write;

/// Caps that keep a large project from swallowing the context window. Anything
/// beyond them is summarised as a count, which is still enough for the model to
/// know the timeline is bigger than what it can see.
const MAX_CLIPS: usize = 60;
const MAX_POOL_ASSETS: usize = 24;
const MAX_NAME_CHARS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Video,
    Audio,
}

impl TrackKind {
    fn tag(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipContext {
    pub id: u64,
    pub name: String,
    pub start: f32,
    pub end: f32,
    pub has_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackContext {
    pub index: usize,
    pub name: String,
    pub kind: TrackKind,
    pub muted: bool,
    pub locked: bool,
    pub clips: Vec<ClipContext>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimelineContext {
    pub duration: f32,
    pub playhead: f32,
    pub tracks: Vec<TrackContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetContext {
    pub name: String,
    pub seconds: f32,
    pub kind: TrackKind,
    pub generated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaPoolContext {
    pub assets: Vec<AssetContext>,
}

/// What the user last pointed at, if anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionContext {
    pub kind: String,
    pub description: String,
    pub clip_id: Option<u64>,
}

impl TimelineContext {
    pub fn clip_count(&self) -> usize {
        self.tracks.iter().map(|track| track.clips.len()).sum()
    }

    /// One line per track, one indented line per clip:
    /// `c12 "intro.mp4" 0.0-6.5 a` — id, name, span, `a` when it has audio.
    pub fn to_llm_context_string(&self) -> String {
        let mut out = String::with_capacity(64 + self.clip_count() * 32);
        let _ = writeln!(
            out,
            "timeline: dur={:.1} head={:.1} tracks={} clips={}",
            self.duration,
            self.playhead,
            self.tracks.len(),
            self.clip_count()
        );

        if self.tracks.is_empty() {
            out.push_str("  (empty)\n");
            return out;
        }

        // The budget is shared across tracks so one long track cannot starve
        // the others of representation.
        let mut remaining = MAX_CLIPS;
        for track in &self.tracks {
            let _ = write!(
                out,
                "t{} {} {}",
                track.index,
                track.name,
                track.kind.tag()
            );
            if track.muted {
                out.push_str(" muted");
            }
            if track.locked {
                out.push_str(" locked");
            }
            out.push('\n');

            let shown = track.clips.len().min(remaining);
            for clip in &track.clips[..shown] {
                let _ = writeln!(
                    out,
                    "  c{} \"{}\" {:.1}-{:.1}{}",
                    clip.id,
                    truncate(&clip.name),
                    clip.start,
                    clip.end,
                    if clip.has_audio { " a" } else { "" }
                );
            }
            if track.clips.len() > shown {
                let _ = writeln!(out, "  +{} more", track.clips.len() - shown);
            }
            remaining -= shown;
        }

        out
    }
}

impl MediaPoolContext {
    /// `"vo.wav" 12.4s audio` per asset, generated ones flagged.
    pub fn to_llm_context_string(&self) -> String {
        if self.assets.is_empty() {
            return "pool: (empty)\n".to_string();
        }

        let shown = self.assets.len().min(MAX_POOL_ASSETS);
        let mut out = String::with_capacity(16 + shown * 28);
        let _ = writeln!(out, "pool: {}", self.assets.len());

        for asset in &self.assets[..shown] {
            let _ = writeln!(
                out,
                "  \"{}\" {:.1}s {}{}",
                truncate(&asset.name),
                asset.seconds,
                asset.kind.tag(),
                if asset.generated { " gen" } else { "" }
            );
        }
        if self.assets.len() > shown {
            let _ = writeln!(out, "  +{} more", self.assets.len() - shown);
        }
        out
    }
}

impl SelectionContext {
    pub fn to_llm_context_string(&self) -> String {
        match self.clip_id {
            Some(id) => format!("selection: {} c{} {}\n", self.kind, id, self.description),
            None => format!("selection: {} {}\n", self.kind, self.description),
        }
    }
}

/// Long names cost tokens and never disambiguate better than their first words.
fn truncate(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_CHARS {
        return name.to_string();
    }
    name.chars().take(MAX_NAME_CHARS - 1).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u64, name: &str, start: f32, end: f32) -> ClipContext {
        ClipContext {
            id,
            name: name.into(),
            start,
            end,
            has_audio: true,
        }
    }

    fn timeline(clips: Vec<ClipContext>) -> TimelineContext {
        TimelineContext {
            duration: 12.0,
            playhead: 3.5,
            tracks: vec![TrackContext {
                index: 0,
                name: "V1".into(),
                kind: TrackKind::Video,
                muted: false,
                locked: false,
                clips,
            }],
        }
    }

    #[test]
    fn timeline_serializes_one_dense_line_per_clip() {
        let rendered = timeline(vec![clip(12, "intro.mp4", 0.0, 6.5)]).to_llm_context_string();

        assert!(rendered.starts_with("timeline: dur=12.0 head=3.5 tracks=1 clips=1"));
        assert!(rendered.contains("t0 V1 video"));
        assert!(rendered.contains("c12 \"intro.mp4\" 0.0-6.5 a"));
        // No file paths, no pixels — just what an instruction can refer to.
        assert!(!rendered.contains('/') && !rendered.contains('\\'));
    }

    #[test]
    fn an_empty_timeline_says_so_instead_of_nothing() {
        let rendered = TimelineContext::default().to_llm_context_string();
        assert!(rendered.contains("(empty)"));
    }

    #[test]
    fn oversized_timelines_are_capped_and_counted() {
        let clips: Vec<ClipContext> = (0..MAX_CLIPS as u64 + 10)
            .map(|i| clip(i, "c.mp4", i as f32, i as f32 + 1.0))
            .collect();
        let rendered = timeline(clips).to_llm_context_string();

        assert!(rendered.contains("+10 more"), "the remainder is summarised");
        assert_eq!(
            rendered.matches("  c").count(),
            MAX_CLIPS,
            "never more than the cap"
        );
        assert!(rendered.contains("clips=70"), "the true total is still stated");
    }

    #[test]
    fn track_flags_are_carried_because_they_change_what_is_audible() {
        let mut context = timeline(vec![]);
        context.tracks[0].muted = true;
        context.tracks[0].locked = true;

        let rendered = context.to_llm_context_string();
        assert!(rendered.contains("t0 V1 video muted locked"));
    }

    #[test]
    fn pool_lists_durations_and_marks_generated_assets() {
        let pool = MediaPoolContext {
            assets: vec![
                AssetContext {
                    name: "vo.wav".into(),
                    seconds: 12.44,
                    kind: TrackKind::Audio,
                    generated: false,
                },
                AssetContext {
                    name: "gen_city.mp4".into(),
                    seconds: 4.0,
                    kind: TrackKind::Video,
                    generated: true,
                },
            ],
        };

        let rendered = pool.to_llm_context_string();
        assert!(rendered.contains("\"vo.wav\" 12.4s audio"));
        assert!(rendered.contains("\"gen_city.mp4\" 4.0s video gen"));
    }

    #[test]
    fn long_names_are_truncated() {
        let rendered = timeline(vec![clip(1, &"x".repeat(120), 0.0, 1.0)]).to_llm_context_string();
        assert!(rendered.contains('…'));
        assert!(rendered.len() < 200);
    }

    #[test]
    fn selection_names_the_clip_it_refers_to() {
        let selection = SelectionContext {
            kind: "clip".into(),
            description: "\"intro.mp4\" on V1".into(),
            clip_id: Some(12),
        };
        assert_eq!(
            selection.to_llm_context_string(),
            "selection: clip c12 \"intro.mp4\" on V1\n"
        );
    }
}
