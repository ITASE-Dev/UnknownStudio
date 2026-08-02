//! The deconstructed competitor video.
//!
//! Everything here is plain data: the warehouse persists it, the diff engine
//! reads it, and neither knows anything about the UI or the timeline.

use serde::{Deserialize, Serialize};

/// One outlier upload and everything measured about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorVideo {
    pub video_id: String,
    pub channel_id: String,
    pub title: String,
    pub view_count: u64,
    /// Views over the channel's own baseline — why this video was pulled in.
    pub outlier_multiplier: f64,
    pub duration_sec: f32,
    pub published_at: Option<String>,
    pub retention: RetentionAnalysis,
    pub audio: AudioDynamics,
    pub structure: VisualAndPacingStructure,
    pub transcript: TranscriptAndHooks,
    /// When this row was deconstructed, unix seconds.
    pub analyzed_at: u64,
}

impl CompetitorVideo {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.video_id)
    }

    /// Moments the edit must be strongest: replay peaks the audience chose.
    pub fn peak_times(&self) -> impl Iterator<Item = f32> + '_ {
        self.retention.peaks.iter().map(|peak| peak.start_sec)
    }
}

// ---------------------------------------------------------------- retention

/// Replay peaks and abandonment, kept side by side — a video is explained by
/// both what pulled viewers back and what pushed them out.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetentionAnalysis {
    pub peaks: Vec<HeatmapPeak>,
    pub drops: Vec<RetentionDrop>,
    /// Share of the video an average viewer watched, 0.0..=1.0.
    pub average_view_ratio: f32,
}

impl RetentionAnalysis {
    /// The strongest replay moment.
    pub fn hottest(&self) -> Option<&HeatmapPeak> {
        self.peaks
            .iter()
            .max_by(|a, b| a.intensity.total_cmp(&b.intensity))
    }

    /// The worst place the audience left.
    pub fn worst_drop(&self) -> Option<&RetentionDrop> {
        self.drops
            .iter()
            .max_by(|a, b| a.severity.total_cmp(&b.severity))
    }

    /// Whether `seconds` falls inside a replay peak.
    pub fn peak_at(&self, seconds: f32) -> Option<&HeatmapPeak> {
        self.peaks
            .iter()
            .find(|peak| seconds >= peak.start_sec && seconds < peak.end_sec)
    }
}

/// A most-replayed span from the YouTube heatmap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapPeak {
    pub start_sec: f32,
    pub end_sec: f32,
    /// Replay intensity relative to the video's own mean, 1.0 being average.
    pub intensity: f32,
    /// What is happening here, from the transcript and the VLM tags.
    pub description: String,
}

impl HeatmapPeak {
    pub fn duration_sec(&self) -> f32 {
        (self.end_sec - self.start_sec).max(0.0)
    }
}

/// Where the audience left, and the best available explanation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionDrop {
    pub start_sec: f32,
    pub end_sec: f32,
    /// Fraction of the remaining audience lost across the span, 0.0..=1.0.
    pub severity: f32,
    pub cause: DropCause,
}

/// Why viewers left. Drives which revision the diff engine proposes, so these
/// are the causes an edit can actually answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropCause {
    /// Long static shot, no cut, no motion.
    PacingStall,
    /// Dead air the edit should have removed.
    DeadAir,
    /// Talking without anything to look at.
    VisualMonotony,
    /// The promise of the hook was not paid off.
    UnmetExpectation,
    /// The ending outstayed its welcome.
    WeakOutro,
    Unclassified,
}

impl DropCause {
    pub fn label(self) -> &'static str {
        match self {
            Self::PacingStall => "pacing stall",
            Self::DeadAir => "dead air",
            Self::VisualMonotony => "visual monotony",
            Self::UnmetExpectation => "unmet expectation",
            Self::WeakOutro => "weak outro",
            Self::Unclassified => "unclassified",
        }
    }

    /// One-line editorial fix, used as the suggestion on a `FixRetentionDrop`.
    pub fn remedy(self) -> &'static str {
        match self {
            Self::PacingStall => "Pacing too slow here — cut silences and add a jump cut.",
            Self::DeadAir => "Remove the dead air; tighten the gap between phrases.",
            Self::VisualMonotony => "Static talking head — cover with B-roll or a reframe.",
            Self::UnmetExpectation => "Pay off the hook's promise before this point.",
            Self::WeakOutro => "Ending drags — cut to the call to action sooner.",
            Self::Unclassified => "Review this span; retention falls without an obvious cause.",
        }
    }
}

// ------------------------------------------------------------------- audio

/// The audio shape of the video: where it gets loud, where it drops out, and
/// what sits on the seams.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioDynamics {
    pub volume_peaks: Vec<VolumeEvent>,
    pub volume_drops: Vec<VolumeEvent>,
    /// Gaps with no speech and no bed — the empty air.
    pub silences: Vec<SilenceGap>,
    /// Sound effects riding the transitions.
    pub transitions: Vec<TransitionSound>,
    /// Programme loudness, for reference when matching a mix.
    pub mean_dbfs: f32,
}

impl AudioDynamics {
    /// Transition sounds per minute — the density a viral mix runs at.
    pub fn sfx_per_minute(&self, duration_sec: f32) -> f32 {
        if duration_sec <= 0.0 {
            return 0.0;
        }
        self.transitions.len() as f32 / (duration_sec / 60.0)
    }

    /// The transition sound closest to `seconds`, within `tolerance`.
    pub fn transition_near(&self, seconds: f32, tolerance: f32) -> Option<&TransitionSound> {
        self.transitions
            .iter()
            .filter(|sfx| (sfx.timestamp_sec - seconds).abs() <= tolerance)
            .min_by(|a, b| {
                (a.timestamp_sec - seconds)
                    .abs()
                    .total_cmp(&(b.timestamp_sec - seconds).abs())
            })
    }
}

/// A loudness excursion, up or down.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeEvent {
    pub timestamp_sec: f32,
    pub dbfs: f32,
    /// Difference from the programme mean; positive is a peak.
    pub delta_db: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilenceGap {
    pub start_sec: f32,
    pub end_sec: f32,
}

impl SilenceGap {
    pub fn duration_sec(&self) -> f32 {
        (self.end_sec - self.start_sec).max(0.0)
    }
}

/// An SFX on a cut. `sfx_type` is the library id the generator resolves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionSound {
    pub timestamp_sec: f32,
    pub sfx_type: String,
    /// Loudness relative to the bed.
    pub gain_db: f32,
}

// ------------------------------------------------------- visual and pacing

/// Cutting rhythm, cutaway coverage and how the video lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisualAndPacingStructure {
    /// Scene boundaries from FFmpeg's `scdet`, in seconds.
    pub scene_cuts: Vec<SceneCut>,
    pub broll: Vec<BRollPlacement>,
    pub ending: VideoEndingAnalysis,
    /// Mean seconds a shot is held.
    pub average_shot_sec: f32,
}

impl VisualAndPacingStructure {
    pub fn cuts_per_minute(&self, duration_sec: f32) -> f32 {
        if duration_sec <= 0.0 {
            return 0.0;
        }
        self.scene_cuts.len() as f32 / (duration_sec / 60.0)
    }

    /// Share of the runtime covered by a cutaway, 0.0..=1.0.
    pub fn broll_coverage(&self, duration_sec: f32) -> f32 {
        if duration_sec <= 0.0 {
            return 0.0;
        }
        let covered: f32 = self.broll.iter().map(BRollPlacement::duration_sec).sum();
        (covered / duration_sec).clamp(0.0, 1.0)
    }

    pub fn broll_at(&self, seconds: f32) -> Option<&BRollPlacement> {
        self.broll
            .iter()
            .find(|shot| seconds >= shot.start_sec && seconds < shot.end_sec)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneCut {
    pub timestamp_sec: f32,
    /// `scdet` confidence, 0.0..=1.0.
    pub score: f32,
}

/// A cutaway the competitor laid over their A-roll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BRollPlacement {
    pub start_sec: f32,
    pub end_sec: f32,
    /// What the shot is of, in words — the text the vector index embeds.
    pub semantic_topic: String,
    /// Whether the presenter appears in it. Decides the identity constraint
    /// when this shot is used as a reference for generation.
    pub features_presenter: bool,
}

impl BRollPlacement {
    pub fn duration_sec(&self) -> f32 {
        (self.end_sec - self.start_sec).max(0.0)
    }
}

/// How the video closes — the part that decides the next view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoEndingAnalysis {
    /// Where the outro begins.
    pub start_sec: f32,
    pub style: EndingStyle,
    /// Seconds between the last substantive beat and the end card.
    pub tail_sec: f32,
    /// Whether the ending loops back to the hook.
    pub loops_to_hook: bool,
    pub call_to_action: Option<String>,
}

impl Default for VideoEndingAnalysis {
    fn default() -> Self {
        Self {
            start_sec: 0.0,
            style: EndingStyle::Unknown,
            tail_sec: 0.0,
            loops_to_hook: false,
            call_to_action: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndingStyle {
    /// Cuts out on the last beat; nothing trails.
    HardCut,
    /// Points at the next video before the energy falls.
    NextVideoTease,
    /// Returns to the opening image or question.
    LoopBack,
    /// Talking-head sign-off.
    Outro,
    Unknown,
}

impl EndingStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::HardCut => "hard cut",
            Self::NextVideoTease => "next-video tease",
            Self::LoopBack => "loop back to hook",
            Self::Outro => "spoken outro",
            Self::Unknown => "unknown",
        }
    }
}

// ------------------------------------------------- transcript, VLM, hooks

/// The spoken track, tagged with what was on screen while it was said.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranscriptAndHooks {
    pub segments: Vec<TranscriptSegment>,
    pub hook: HookAnalysis,
    pub language: Option<String>,
}

impl TranscriptAndHooks {
    pub fn text(&self) -> String {
        self.segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn segment_at(&self, seconds: f32) -> Option<&TranscriptSegment> {
        self.segments
            .iter()
            .find(|s| seconds >= s.start_sec && seconds < s.end_sec)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start_sec: f32,
    pub end_sec: f32,
    pub text: String,
    /// What the VLM saw while this was spoken.
    pub visual: VlmTag,
}

/// Shot type, as labelled by a vision-language model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VlmTag {
    TalkingHead,
    ScreenRecord,
    BRoll,
    TextOnScreen,
    Archival,
    Animation,
    Unknown,
}

impl VlmTag {
    pub fn label(self) -> &'static str {
        match self {
            Self::TalkingHead => "talking head",
            Self::ScreenRecord => "screen record",
            Self::BRoll => "b-roll",
            Self::TextOnScreen => "text on screen",
            Self::Archival => "archival",
            Self::Animation => "animation",
            Self::Unknown => "unknown",
        }
    }

    /// Whether this shot type carries the presenter's face, which is what
    /// makes generated coverage subject to the identity constraint.
    pub fn shows_presenter(self) -> bool {
        matches!(self, Self::TalkingHead)
    }
}

/// The opening seconds, taken apart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookAnalysis {
    /// Seconds before the first substantive claim.
    pub time_to_value_sec: f32,
    pub hook_type: HookType,
    /// The opening line itself.
    pub opening_line: String,
    /// Words per minute over the hook window.
    pub words_per_minute: f32,
    /// Cuts inside the hook — how hard the opening is edited.
    pub cuts_in_hook: usize,
    /// The promise the hook makes, in the model's words. Embedded for search.
    pub promise: String,
}

impl Default for HookAnalysis {
    fn default() -> Self {
        Self {
            time_to_value_sec: 0.0,
            hook_type: HookType::Unknown,
            opening_line: String::new(),
            words_per_minute: 0.0,
            cuts_in_hook: 0,
            promise: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookType {
    /// Shows the payoff first, then rewinds.
    ColdOpen,
    Question,
    /// States a contrarian or surprising claim.
    Provocation,
    /// Promises a specific outcome by the end.
    Promise,
    Story,
    Unknown,
}

impl HookType {
    pub fn label(self) -> &'static str {
        match self {
            Self::ColdOpen => "cold open",
            Self::Question => "question",
            Self::Provocation => "provocation",
            Self::Promise => "promise",
            Self::Story => "story",
            Self::Unknown => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(start: f32, end: f32, intensity: f32) -> HeatmapPeak {
        HeatmapPeak {
            start_sec: start,
            end_sec: end,
            intensity,
            description: String::new(),
        }
    }

    #[test]
    fn the_hottest_peak_is_the_most_replayed_not_the_first() {
        let retention = RetentionAnalysis {
            peaks: vec![peak(0.0, 5.0, 1.4), peak(30.0, 35.0, 3.2), peak(60.0, 65.0, 2.1)],
            drops: Vec::new(),
            average_view_ratio: 0.5,
        };
        assert_eq!(retention.hottest().map(|p| p.start_sec), Some(30.0));
    }

    #[test]
    fn a_peak_is_found_by_the_span_it_covers() {
        let retention = RetentionAnalysis {
            peaks: vec![peak(10.0, 20.0, 2.0)],
            drops: Vec::new(),
            average_view_ratio: 0.5,
        };

        assert!(retention.peak_at(15.0).is_some());
        assert!(retention.peak_at(9.9).is_none(), "before the span");
        assert!(retention.peak_at(20.0).is_none(), "the end is exclusive");
    }

    #[test]
    fn broll_coverage_is_a_share_of_the_runtime_and_never_exceeds_it() {
        let structure = VisualAndPacingStructure {
            scene_cuts: Vec::new(),
            broll: vec![
                BRollPlacement {
                    start_sec: 0.0,
                    end_sec: 15.0,
                    semantic_topic: "city timelapse".into(),
                    features_presenter: false,
                },
                BRollPlacement {
                    start_sec: 20.0,
                    end_sec: 35.0,
                    semantic_topic: "desk setup".into(),
                    features_presenter: false,
                },
            ],
            ending: VideoEndingAnalysis::default(),
            average_shot_sec: 3.0,
        };

        assert!((structure.broll_coverage(60.0) - 0.5).abs() < 1e-5);
        // A bad duration must not produce a coverage above 1.
        assert!(structure.broll_coverage(10.0) <= 1.0);
        assert_eq!(structure.broll_coverage(0.0), 0.0);
    }

    #[test]
    fn the_nearest_transition_wins_and_distant_ones_are_ignored() {
        let audio = AudioDynamics {
            transitions: vec![
                TransitionSound { timestamp_sec: 10.0, sfx_type: "whoosh".into(), gain_db: -6.0 },
                TransitionSound { timestamp_sec: 12.0, sfx_type: "riser".into(), gain_db: -8.0 },
            ],
            ..Default::default()
        };

        assert_eq!(
            audio.transition_near(11.4, 1.0).map(|s| s.sfx_type.as_str()),
            Some("riser")
        );
        assert!(audio.transition_near(30.0, 1.0).is_none());
    }

    #[test]
    fn every_drop_cause_offers_an_editorial_remedy() {
        let causes = [
            DropCause::PacingStall,
            DropCause::DeadAir,
            DropCause::VisualMonotony,
            DropCause::UnmetExpectation,
            DropCause::WeakOutro,
            DropCause::Unclassified,
        ];
        for cause in causes {
            assert!(!cause.remedy().is_empty(), "{} has no remedy", cause.label());
        }
    }

    #[test]
    fn the_model_survives_a_json_round_trip() {
        let video = CompetitorVideo {
            video_id: "abc".into(),
            channel_id: "UC1".into(),
            title: "t".into(),
            view_count: 10,
            outlier_multiplier: 3.0,
            duration_sec: 60.0,
            published_at: None,
            retention: RetentionAnalysis::default(),
            audio: AudioDynamics::default(),
            structure: VisualAndPacingStructure::default(),
            transcript: TranscriptAndHooks::default(),
            analyzed_at: 0,
        };

        let json = serde_json::to_string(&video).expect("serialize");
        let back: CompetitorVideo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.video_id, "abc");
        assert_eq!(back.structure.ending.style, EndingStyle::Unknown);
    }
}
