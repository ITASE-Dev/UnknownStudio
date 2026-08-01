//! What the user right-clicked, and how to describe it to the assistant.

use std::path::PathBuf;

/// A thing on screen that can carry a context menu.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaSelection {
    /// A block on a timeline track.
    Clip {
        id: u64,
        label: String,
        track: String,
        start_seconds: f32,
        duration_seconds: f32,
    },
    /// A whole audio lane.
    AudioTrack { index: usize, name: String },
    /// The program monitor at the current playhead.
    PreviewScreen { seconds: f32 },
    /// An asset in the media pool.
    PoolAsset {
        name: String,
        path: Option<PathBuf>,
        generated: bool,
    },
}

impl MediaSelection {
    /// Short label for the menu header.
    pub fn title(&self) -> String {
        match self {
            Self::Clip { label, track, .. } => format!("{label} · {track}"),
            Self::AudioTrack { name, .. } => format!("Track {name}"),
            Self::PreviewScreen { .. } => "Program monitor".to_string(),
            Self::PoolAsset { name, .. } => name.clone(),
        }
    }

    /// One line of context handed to the assistant. Only facts that are on
    /// screen — the model must never be told a timecode nobody measured.
    pub fn context_line(&self) -> String {
        match self {
            Self::Clip {
                label,
                track,
                start_seconds,
                duration_seconds,
                ..
            } => format!(
                "Selected clip “{label}” on {track}, {} → {} ({:.1}s long).",
                timecode(*start_seconds),
                timecode(start_seconds + duration_seconds),
                duration_seconds
            ),
            Self::AudioTrack { name, .. } => {
                format!("Selected audio track {name}.")
            }
            Self::PreviewScreen { seconds } => {
                format!("Looking at the program monitor at {}.", timecode(*seconds))
            }
            Self::PoolAsset {
                name, generated, ..
            } => {
                let origin = if *generated { "generated" } else { "imported" };
                format!("Selected {origin} media pool asset “{name}”.")
            }
        }
    }

    /// Clip id, when the selection is one.
    pub fn clip_id(&self) -> Option<u64> {
        match self {
            Self::Clip { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// Whether removing this selection is meaningful.
    pub fn is_removable(&self) -> bool {
        matches!(self, Self::Clip { .. })
    }

    /// Whether it sits on the timeline and can be cut.
    pub fn is_splittable(&self) -> bool {
        matches!(self, Self::Clip { .. })
    }
}

fn timecode(seconds: f32) -> String {
    let total = seconds.max(0.0) as i32;
    format!("{:02}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> MediaSelection {
        MediaSelection::Clip {
            id: 7,
            label: "a.mp4".into(),
            track: "V1".into(),
            start_seconds: 64.0,
            duration_seconds: 6.5,
        }
    }

    #[test]
    fn clip_context_states_only_measured_facts() {
        let line = clip().context_line();
        assert!(line.contains("a.mp4"), "names the clip");
        assert!(line.contains("01:04"), "start timecode");
        assert!(line.contains("01:10"), "end timecode");
        assert!(line.contains("6.5s"));
    }

    #[test]
    fn capabilities_follow_the_selection_kind() {
        assert_eq!(clip().clip_id(), Some(7));
        assert!(clip().is_removable() && clip().is_splittable());

        let preview = MediaSelection::PreviewScreen { seconds: 12.0 };
        assert_eq!(preview.clip_id(), None);
        assert!(!preview.is_removable() && !preview.is_splittable());
        assert!(preview.context_line().contains("00:12"));
    }

    #[test]
    fn pool_assets_report_their_origin() {
        let generated = MediaSelection::PoolAsset {
            name: "gen_city.mp4".into(),
            path: None,
            generated: true,
        };
        assert!(generated.context_line().contains("generated"));
        assert_eq!(generated.title(), "gen_city.mp4");
    }
}
