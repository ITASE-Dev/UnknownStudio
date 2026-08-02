//! Competitor dashboard: find a channel's outliers, take one apart, then diff
//! it against the open studio project.
//!
//! Every stage runs on a background runtime; the view polls for results and
//! never blocks a frame.

use crate::ai_tooling::competitor::models::CompetitorVideo;
use crate::ai_tooling::competitor::VlmTag;
use crate::ai_tooling::pipeline::CompetitorDNA;
use crate::app::orchestrator::{AppEvent, Prerequisites};
use crate::ai_tooling::revision::models::RevisionPlan;
use crate::ai_tooling::revision::timeline::CurrentTimelineState;
use crate::ai_tooling::youtube_insights::{OutlierAnalysis, PacingHeatmap, ViralScore};
use crate::app::router::AppRoute;
use crate::ui::core::buttons::{ghost_button, pro_button};
use crate::ui::core::inputs::pro_text_input;
use crate::ui::core::typography::{hairline_rule, panel, property_row, section_header};
use crate::ui::theme::tokens::*;
use crate::ai_tooling::youtube_insights::heatmap::HOOK_WINDOW_SEC;
use eframe::egui::{
    self, Align, Align2, FontFamily, FontId, Layout, Pos2, Rect, RichText, ScrollArea, Sense,
    Stroke, Ui, Vec2,
};
use std::collections::HashMap;

/// Half-width of the transcript window the scraper measures around the peak.
const PEAK_WINDOW_SEC: f64 = 10.0;

/// Fallback runtime when the API did not report one.
const ASSUMED_DURATION_SEC: f32 = 300.0;

/// What the view is waiting on. Set when a request is queued, cleared by the
/// terminal event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum Busy {
    #[default]
    Idle,
    Channel,
    Pacing,
    Deconstructing(String),
    Comparing,
}

impl Busy {
    fn label(&self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Channel => "Sampling uploads…",
            Self::Pacing => "Measuring pacing…",
            Self::Deconstructing(_) => "Deconstructing — retention, audio, cuts, hooks…",
            Self::Comparing => "Comparing against your timeline…",
        }
    }
}

/// One thing the view wants done. The app turns these into `AppCommand`s.
///
/// The view holds no runtime and no channels: it asks, absorbs events, and
/// draws. That is the whole of its relationship with the background.
#[derive(Debug, Clone)]
pub enum InsightsRequest {
    CheckPrerequisites,
    AnalyzeChannel(String),
    Deconstruct {
        score: Box<ViralScore>,
        channel_id: String,
        duration_sec: f32,
    },
    MeasurePacing(String),
    ComparePlan {
        video_id: String,
        use_llm: bool,
        presenter_reference: Option<String>,
    },
}

#[derive(Default)]
pub struct InsightsState {
    /// `UC…` channel id, as the API wants it.
    pub channel_id: String,
    pub analysis: Option<OutlierAnalysis>,
    pub heatmap: Option<PacingHeatmap>,
    pub error: Option<String>,
    /// Deconstructed videos by id, so the dashboard can render them without
    /// going back to the warehouse each frame.
    pub deconstructed: HashMap<String, CompetitorVideo>,
    /// Which deconstructed video the deep panel is showing.
    pub selected: Option<String>,
    /// The user's own face reference, for generated shots that need it.
    pub presenter_reference: String,
    /// Route the comparison through the three-agent LLM pipeline instead of
    /// the deterministic diff engine.
    pub use_llm_director: bool,
    /// Queued requests, drained by the app once per frame.
    pub outbox: Vec<InsightsRequest>,
    /// Progress from the worker, 0.0..=1.0, with the stage name.
    pub progress: Option<(f32, String)>,
    /// What the analysis features can do right now. `None` until the
    /// preflight job comes back.
    pub prerequisites: Option<Prerequisites>,
    /// Set once the preflight has been asked for, so it is not re-queued
    /// on every frame the screen is open.
    preflight_requested: bool,
    /// The LLM's reading of the selected video, when the AI path ran.
    ///
    /// Agent 1's output was previously emitted and dropped: the call was
    /// paid for and the answer thrown away.
    pub dna: Option<CompetitorDNA>,
    durations: HashMap<String, f32>,
    busy: Busy,
}

impl InsightsState {
    pub fn is_busy(&self) -> bool {
        self.busy != Busy::Idle
    }

    fn duration_of(&self, video_id: &str) -> f32 {
        self.durations
            .get(video_id)
            .copied()
            .unwrap_or(ASSUMED_DURATION_SEC)
    }

    /// Everything queued since the last call.
    pub fn take_requests(&mut self) -> Vec<InsightsRequest> {
        std::mem::take(&mut self.outbox)
    }

    /// Asks for the preflight once, the first time the screen is drawn.
    pub fn request_preflight(&mut self) {
        if self.preflight_requested {
            return;
        }
        self.preflight_requested = true;
        self.outbox.push(InsightsRequest::CheckPrerequisites);
    }

    /// Re-runs the preflight, after the user has edited their keys.
    pub fn recheck(&mut self) {
        self.preflight_requested = false;
        self.request_preflight();
    }

    /// Queues a channel sample.
    pub fn analyze_channel(&mut self) {
        let channel_id = self.channel_id.trim().to_string();
        if channel_id.is_empty() {
            self.error = Some("Enter a channel id (the UC… one, not an @handle).".into());
            return;
        }

        // Credentials are the worker's problem: checking them here would mean
        // loading `.env` on the render thread every time the button is pressed.
        self.error = None;
        self.analysis = None;
        self.heatmap = None;
        self.busy = Busy::Channel;
        self.outbox.push(InsightsRequest::AnalyzeChannel(channel_id));
    }

    pub fn measure_pacing(&mut self, video_id: &str) {
        self.error = None;
        self.heatmap = None;
        self.busy = Busy::Pacing;
        self.outbox
            .push(InsightsRequest::MeasurePacing(video_id.to_string()));
    }

    pub fn deconstruct_video(&mut self, score: &ViralScore) {
        self.error = None;
        self.busy = Busy::Deconstructing(score.video_id.clone());
        self.outbox.push(InsightsRequest::Deconstruct {
            duration_sec: self.duration_of(&score.video_id),
            score: Box::new(score.clone()),
            channel_id: self.channel_id.trim().to_string(),
        });
    }

    /// Queues the diff. The worker reads the timeline itself, so nothing about
    /// the edit needs to be captured here.
    pub fn compare(&mut self, video_id: &str) {
        if !self.deconstructed.contains_key(video_id) {
            self.error = Some("Deconstruct the video before comparing.".into());
            return;
        }

        let reference = self.presenter_reference.trim().to_string();
        self.error = None;
        self.busy = Busy::Comparing;
        self.outbox.push(InsightsRequest::ComparePlan {
            video_id: video_id.to_string(),
            use_llm: self.use_llm_director,
            presenter_reference: (!reference.is_empty()).then_some(reference),
        });
    }

    /// Absorbs one event from the orchestrator.
    ///
    /// Returns a finished plan for the caller to hand to the studio — this view
    /// has no path to the editor and should not grow one.
    pub fn apply_event(&mut self, event: &AppEvent) -> Option<RevisionPlan> {
        match event {
            AppEvent::AnalysisProgress { fraction, stage, .. } => {
                self.progress = Some((*fraction, stage.clone()));
            }
            AppEvent::OutliersReady {
                analysis,
                durations,
                ..
            } => {
                self.analysis = Some((**analysis).clone());
                self.durations = durations.clone();
            }
            AppEvent::VideoDeconstructed { video, .. } => {
                self.selected = Some(video.video_id.clone());
                self.deconstructed
                    .insert(video.video_id.clone(), (**video).clone());
            }
            AppEvent::PrerequisitesChecked { report, .. } => {
                self.prerequisites = Some(report.clone())
            }
            AppEvent::DnaReady { dna, .. } => self.dna = Some((**dna).clone()),
            AppEvent::PacingReady { heatmap, .. } => self.heatmap = Some((**heatmap).clone()),
            AppEvent::RevisionsReady { plan, .. } => return Some((**plan).clone()),
            AppEvent::Error { message, .. } => self.error = Some(message.clone()),
            AppEvent::Finished { .. } => {
                self.busy = Busy::Idle;
                self.progress = None;
            }
            _ => {}
        }
        None
    }
}

/// What the view wants the app to do once the frame is drawn.
#[derive(Default)]
pub struct InsightsOutcome {
    /// The user asked to compare against the open project.
    pub compare_with_studio: Option<String>,
}

pub fn show(
    ctx: &egui::Context,
    route: &mut AppRoute,
    state: &mut InsightsState,
    project_open: bool,
) -> InsightsOutcome {
    let mut outcome = InsightsOutcome::default();
    super::page(ctx, 1100.0, |ui| {
        content(ui, route, state, project_open, &mut outcome)
    });
    outcome
}

fn content(
    ui: &mut Ui,
    route: &mut AppRoute,
    state: &mut InsightsState,
    project_open: bool,
    outcome: &mut InsightsOutcome,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Competitor research")
                .heading()
                .strong()
                .color(TEXT_PRIMARY),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ghost_button(ui, "Back").clicked() {
                *route = AppRoute::Dashboard;
            }
        });
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Finds the uploads that beat a channel's own baseline, takes one apart, and turns \
             the difference into an edit plan.",
        )
        .small()
        .color(TEXT_SECONDARY),
    );
    ui.add_space(10.0);
    hairline_rule(ui);
    ui.add_space(12.0);

    state.request_preflight();
    preflight_panel(ui, state);
    ui.add_space(10.0);

    search_row(ui, state);

    if let Some(error) = &state.error {
        ui.add_space(8.0);
        ui.label(RichText::new(error).small().color(ERR));
    }

    let mut measure: Option<String> = None;
    let mut take_apart: Option<ViralScore> = None;
    if let Some(analysis) = &state.analysis {
        ui.add_space(12.0);
        let busy = state.is_busy();
        let known = &state.deconstructed;
        section_header(ui, "Outliers");
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} uploads · baseline {} views · {} golden",
                    analysis.sample_size,
                    thousands(analysis.baseline_views),
                    analysis.golden().count()
                ))
                .small()
                .color(TEXT_SECONDARY),
            );
            if !analysis.reliable {
                ui.label(
                    RichText::new("small sample — treat the verdict as indicative")
                        .small()
                        .color(WARN),
                );
            }
        });
        ui.add_space(8.0);

        ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(280.0)
            .id_source("insights_results")
            .show(ui, |ui| {
                for score in analysis.scores.iter().take(25) {
                    let done = known.contains_key(&score.video_id);
                    match row(ui, score, busy, done) {
                        Some(RowAction::Pace) => measure = Some(score.video_id.clone()),
                        Some(RowAction::Deconstruct) => take_apart = Some(score.clone()),
                        None => {}
                    }
                }
            });
    }
    if let Some(video_id) = measure {
        state.measure_pacing(&video_id);
    }
    if let Some(score) = take_apart {
        state.deconstruct_video(&score);
    }

    if let Some(heatmap) = &state.heatmap {
        ui.add_space(12.0);
        pacing(ui, heatmap);
    }

    if let Some(selected) = state.selected.clone() {
        if let Some(video) = state.deconstructed.get(&selected) {
            ui.add_space(12.0);
            deep_analysis(ui, video);
        }
        if let Some(dna) = &state.dna {
            ui.add_space(12.0);
            dna_panel(ui, dna);
        }
        ui.add_space(10.0);
        compare_row(ui, state, project_open, &selected, outcome);
    }
}

fn search_row(ui: &mut Ui, state: &mut InsightsState) {
    panel(ui, |ui| {
        ui.label(RichText::new("Channel id").small().color(TEXT_SECONDARY));
        ui.horizontal(|ui| {
            let field = ui
                .scope(|ui| {
                    ui.set_max_width(360.0);
                    pro_text_input(ui, &mut state.channel_id, "UCxxxxxxxxxxxxxxxxxxxxxx")
                })
                .inner;

            let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_enabled_ui(!state.is_busy(), |ui| {
                if pro_button(ui, "Examine", true).clicked() || submitted {
                    state.analyze_channel();
                }
            });

            if state.is_busy() {
                ui.label(RichText::new(state.busy.label()).small().color(ACCENT));
                if let Some((fraction, stage)) = &state.progress {
                    ui.add(
                        egui::ProgressBar::new(*fraction)
                            .desired_width(150.0)
                            .text(RichText::new(stage).small()),
                    );
                }
            }
        });
    });
}

enum RowAction {
    Pace,
    Deconstruct,
}

fn row(ui: &mut Ui, score: &ViralScore, busy: bool, deconstructed: bool) -> Option<RowAction> {
    let mut action = None;

    panel(ui, |ui| {
        ui.horizontal(|ui| {
            let colour = if score.is_outlier { OK } else { TEXT_PRIMARY };
            ui.add(egui::Label::new(RichText::new(&score.title).color(colour)).truncate(true));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    let label = if deconstructed { "Re-analyse" } else { "Deconstruct" };
                    if pro_button(ui, label, score.is_outlier).clicked() {
                        action = Some(RowAction::Deconstruct);
                    }
                    if ghost_button(ui, "Pacing").clicked() {
                        action = Some(RowAction::Pace);
                    }
                });
                ui.label(
                    RichText::new(format!("{:.1}x", score.multiplier))
                        .strong()
                        .color(if score.is_outlier { OK } else { TEXT_DISABLED }),
                );
                ui.label(
                    RichText::new(format!("{} views", thousands(score.view_count as f64)))
                        .small()
                        .color(TEXT_SECONDARY),
                );
            });
        });
        ui.label(RichText::new(score.reason()).small().color(TEXT_DISABLED));
        if deconstructed {
            ui.label(RichText::new("in the warehouse").small().color(OK));
        }
    });
    ui.add_space(4.0);

    action
}


/// Blends two colours. egui 0.27 has no stable helper for this.
fn lerp_color(from: egui::Color32, to: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    egui::Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

/// What is set up and what is not, before anything is clicked.
fn preflight_panel(ui: &mut Ui, state: &mut InsightsState) {
    let Some(report) = state.prerequisites.clone() else {
        panel(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Checking what is available\u{2026}")
                        .small()
                        .color(TEXT_SECONDARY),
                );
            });
        });
        return;
    };

    panel(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            requirement(ui, "YouTube API key", report.youtube_key);
            requirement(ui, "yt-dlp", report.yt_dlp);
            requirement(ui, "LLM key", report.llm_key);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ghost_button(ui, "Re-check").clicked() {
                    state.recheck();
                }
            });
        });

        let blockers = report.blockers();
        if !blockers.is_empty() {
            ui.add_space(6.0);
            for line in &blockers {
                ui.label(RichText::new(format!("\u{2022} {line}")).small().color(WARN));
            }
            ui.label(
                RichText::new(
                    "Keys go in Settings (the gear on the Projects screen). yt-dlp must be \
                     installed and on PATH.",
                )
                .small()
                .color(TEXT_DISABLED),
            );
        }
        if let Some(error) = &report.config_error {
            ui.add_space(4.0);
            ui.label(RichText::new(error).small().color(TEXT_DISABLED));
        }
    });
}

fn requirement(ui: &mut Ui, label: &str, ok: bool) {
    let (mark, color) = if ok { ("\u{2713}", OK) } else { ("\u{2717}", ERR) };
    ui.label(RichText::new(format!("{mark} {label}")).small().color(color));
    ui.add_space(10.0);
}

/// The pacing heatmap, drawn.
///
/// One bar per measured window: height is the speaking rate, and the dim
/// under-bar is the share of the window that was silent. The hook window is
/// outlined, because a front-loaded opening is the single thing this chart
/// exists to make visible at a glance.
fn heatmap_chart(ui: &mut Ui, heatmap: &PacingHeatmap) {
    const HEIGHT: f32 = 68.0;

    if heatmap.windows.is_empty() {
        ui.label(
            RichText::new("No windows measured \u{2014} the transcript was too short.")
                .small()
                .color(TEXT_DISABLED),
        );
        return;
    }

    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, R_SM, BG_SUNKEN);

    let peak_wpm = heatmap
        .windows
        .iter()
        .map(|w| w.words_per_minute)
        .fold(1.0_f32, f32::max);
    let count = heatmap.windows.len() as f32;
    let slot = rect.width() / count;
    let gap = (slot * 0.15).min(2.0);

    let mut hovered: Option<&crate::ai_tooling::youtube_insights::models::PacingWindow> = None;

    for (index, window) in heatmap.windows.iter().enumerate() {
        let left = rect.left() + index as f32 * slot;
        let bar = Rect::from_min_max(
            Pos2::new(left + gap, rect.bottom() - HEIGHT * (window.words_per_minute / peak_wpm)),
            Pos2::new(left + slot - gap, rect.bottom()),
        );

        // Faster than the overall rate reads warm, slower reads cool: the eye
        // finds the hook without reading a single number.
        let hot = (window.words_per_minute / peak_wpm).clamp(0.0, 1.0);
        let color = lerp_color(ACCENT, WARN, hot);
        painter.rect_filled(bar, R_SM, color.gamma_multiply(0.55 + hot * 0.45));

        // Silence, as a dim cap on the same column.
        if window.silence_ratio > 0.05 {
            let quiet = Rect::from_min_max(
                Pos2::new(bar.left(), rect.top()),
                Pos2::new(bar.right(), rect.top() + HEIGHT * window.silence_ratio * 0.4),
            );
            painter.rect_filled(quiet, R_SM, TEXT_DISABLED.gamma_multiply(0.35));
        }

        if response
            .hover_pos()
            .is_some_and(|pos| pos.x >= bar.left() && pos.x <= bar.right())
        {
            painter.rect_stroke(bar, R_SM, Stroke::new(1.0_f32, TEXT_PRIMARY));
            hovered = Some(window);
        }
    }

    // The hook window, outlined across however many bars it spans.
    let hook_end = (HOOK_WINDOW_SEC / heatmap.duration_sec.max(0.001)).clamp(0.0, 1.0);
    if hook_end > 0.0 {
        let hook = Rect::from_min_max(
            rect.left_top(),
            Pos2::new(rect.left() + rect.width() * hook_end, rect.bottom()),
        );
        painter.rect_stroke(hook, R_SM, Stroke::new(1.0_f32, AI));
        painter.text(
            hook.left_top() + Vec2::new(4.0, 2.0),
            Align2::LEFT_TOP,
            "hook",
            FontId::new(9.0, FontFamily::Proportional),
            AI,
        );
    }

    ui.add_space(2.0);
    match hovered {
        Some(window) => ui.label(
            RichText::new(format!(
                "{:.0}s\u{2013}{:.0}s \u{00b7} {:.0} wpm \u{00b7} {:.0}% silent",
                window.start_sec,
                window.end_sec,
                window.words_per_minute,
                window.silence_ratio * 100.0
            ))
            .small()
            .color(TEXT_SECONDARY),
        ),
        None => ui.label(
            RichText::new(format!(
                "{} windows \u{00b7} peak {peak_wpm:.0} wpm \u{00b7} hover a bar for detail",
                heatmap.windows.len()
            ))
            .small()
            .color(TEXT_DISABLED),
        ),
    };
}

/// Replay peaks and audience drops along the competitor's runtime.
fn retention_strip(ui: &mut Ui, video: &CompetitorVideo) {
    const HEIGHT: f32 = 34.0;

    let duration = video.duration_sec.max(0.001);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, R_SM, BG_SUNKEN);

    let at = |seconds: f32| rect.left() + rect.width() * (seconds / duration).clamp(0.0, 1.0);
    let mut label: Option<String> = None;

    // Drops first, so a peak sitting inside one still reads on top.
    for drop in &video.retention.drops {
        let band = Rect::from_min_max(
            Pos2::new(at(drop.start_sec), rect.top()),
            Pos2::new(at(drop.end_sec).max(at(drop.start_sec) + 2.0), rect.bottom()),
        );
        painter.rect_filled(band, R_SM, ERR.gamma_multiply(0.35));
        if response.hover_pos().is_some_and(|p| (band.left()..=band.right()).contains(&p.x)) {
            label = Some(format!(
                "{:.0}s \u{2014} lost {:.0}% to {}",
                drop.start_sec,
                drop.severity * 100.0,
                drop.cause.label()
            ));
        }
    }

    let hottest = video
        .retention
        .peaks
        .iter()
        .map(|p| p.intensity)
        .fold(1.0_f32, f32::max);

    for peak in &video.retention.peaks {
        let share = (peak.intensity / hottest).clamp(0.15, 1.0);
        let band = Rect::from_min_max(
            Pos2::new(at(peak.start_sec), rect.bottom() - HEIGHT * share),
            Pos2::new(at(peak.end_sec).max(at(peak.start_sec) + 3.0), rect.bottom()),
        );
        painter.rect_filled(band, R_SM, OK.gamma_multiply(0.5 + share * 0.4));
        if response.hover_pos().is_some_and(|p| (band.left()..=band.right()).contains(&p.x)) {
            label = Some(format!(
                "{:.0}s \u{2014} {:.1}x replays \u{00b7} {}",
                peak.start_sec, peak.intensity, peak.description
            ));
        }
    }

    ui.add_space(2.0);
    ui.label(
        RichText::new(label.unwrap_or_else(|| {
            format!(
                "{:.0}s runtime \u{00b7} green = replayed, red = abandoned \u{00b7} hover for detail",
                duration
            )
        }))
        .small()
        .color(TEXT_DISABLED),
    );
}

/// Everything the warehouse holds about one competitor video.
fn deep_analysis(ui: &mut Ui, video: &CompetitorVideo) {
    section_header(ui, "Deconstruction");
    ui.label(
        RichText::new(&video.title)
            .strong()
            .color(TEXT_PRIMARY),
    );
    ui.add_space(6.0);
    panel(ui, |ui| {
        ui.label(RichText::new("Retention").small().strong().color(TEXT_PRIMARY));
        ui.add_space(4.0);
        retention_strip(ui, video);
    });
    ui.add_space(8.0);

    ui.columns(2, |columns| {
        let ui = &mut columns[0];
        panel(ui, |ui| {
            ui.label(RichText::new("Retention").small().strong().color(TEXT_PRIMARY));
            ui.add_space(4.0);
            property_row(
                ui,
                "Average watched",
                &format!("{:.0}%", video.retention.average_view_ratio * 100.0),
            );
            for peak in &video.retention.peaks {
                property_row(
                    ui,
                    &format!("Peak {:.0}s", peak.start_sec),
                    &format!("{:.1}x replays", peak.intensity),
                );
                ui.label(
                    RichText::new(&peak.description)
                        .small()
                        .color(TEXT_DISABLED),
                );
            }
            ui.add_space(4.0);
            for drop in &video.retention.drops {
                property_row(
                    ui,
                    &format!("Drop {:.0}s", drop.start_sec),
                    &format!("−{:.0}% · {}", drop.severity * 100.0, drop.cause.label()),
                );
            }
        });

        let ui = &mut columns[1];
        panel(ui, |ui| {
            ui.label(RichText::new("Audio").small().strong().color(TEXT_PRIMARY));
            ui.add_space(4.0);
            property_row(ui, "Programme loudness", &format!("{:.1} dBFS", video.audio.mean_dbfs));
            property_row(
                ui,
                "Transition SFX",
                &format!(
                    "{} · {:.1}/min",
                    video.audio.transitions.len(),
                    video.audio.sfx_per_minute(video.duration_sec)
                ),
            );
            property_row(ui, "Volume peaks", &video.audio.volume_peaks.len().to_string());
            property_row(ui, "Volume drops", &video.audio.volume_drops.len().to_string());
            property_row(
                ui,
                "Silences",
                &format!(
                    "{} · longest {:.1}s",
                    video.audio.silences.len(),
                    video
                        .audio
                        .silences
                        .iter()
                        .map(|s| s.duration_sec())
                        .fold(0.0_f32, f32::max)
                ),
            );
        });
    });

    ui.add_space(8.0);
    ui.columns(2, |columns| {
        let ui = &mut columns[0];
        panel(ui, |ui| {
            ui.label(
                RichText::new("Pacing & structure")
                    .small()
                    .strong()
                    .color(TEXT_PRIMARY),
            );
            ui.add_space(4.0);
            property_row(
                ui,
                "Cuts",
                &format!(
                    "{:.1}/min · {:.1}s average shot",
                    video.structure.cuts_per_minute(video.duration_sec),
                    video.structure.average_shot_sec
                ),
            );
            property_row(
                ui,
                "B-roll coverage",
                &format!(
                    "{:.0}% · {} shots",
                    video.structure.broll_coverage(video.duration_sec) * 100.0,
                    video.structure.broll.len()
                ),
            );
            ui.add_space(4.0);
            ui.label(RichText::new("Ending").small().color(TEXT_SECONDARY));
            property_row(ui, "Style", video.structure.ending.style.label());
            property_row(
                ui,
                "Starts / tail",
                &format!(
                    "{:.0}s · {:.1}s tail",
                    video.structure.ending.start_sec, video.structure.ending.tail_sec
                ),
            );
            if let Some(cta) = &video.structure.ending.call_to_action {
                ui.label(RichText::new(cta).small().color(TEXT_DISABLED));
            }
        });

        let ui = &mut columns[1];
        panel(ui, |ui| {
            ui.label(
                RichText::new("Hook & transcript")
                    .small()
                    .strong()
                    .color(TEXT_PRIMARY),
            );
            ui.add_space(4.0);
            let hook = &video.transcript.hook;
            property_row(ui, "Type", hook.hook_type.label());
            property_row(ui, "Time to value", &format!("{:.1}s", hook.time_to_value_sec));
            property_row(ui, "Rate", &format!("{:.0} wpm", hook.words_per_minute));
            property_row(ui, "Cuts in hook", &hook.cuts_in_hook.to_string());
            ui.add_space(2.0);
            ui.label(RichText::new(&hook.opening_line).small().italics().color(TEXT_SECONDARY));
            ui.add_space(4.0);
            property_row(ui, "Segments", &video.transcript.segments.len().to_string());
            ui.label(
                RichText::new(format!("Shot mix: {}", shot_mix(video)))
                    .small()
                    .color(TEXT_DISABLED),
            );
        });
    });
}

/// The VLM tag distribution, as a readable line.
fn shot_mix(video: &CompetitorVideo) -> String {
    let mut counts: Vec<(VlmTag, usize)> = Vec::new();
    for segment in &video.transcript.segments {
        match counts.iter_mut().find(|(tag, _)| *tag == segment.visual) {
            Some((_, count)) => *count += 1,
            None => counts.push((segment.visual, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));

    counts
        .iter()
        .map(|(tag, count)| format!("{} ×{count}", tag.label()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Agent 1's reading of the reference, when the AI director ran.
fn dna_panel(ui: &mut Ui, dna: &CompetitorDNA) {
    section_header(ui, "AI reading");
    panel(ui, |ui| {
        ui.label(RichText::new(&dna.verdict).small().color(TEXT_SECONDARY));
        ui.add_space(6.0);
        property_row(ui, "Hook", &format!("{} — {}", dna.hook.hook_type, dna.hook.promise));
        property_row(
            ui,
            "Pacing",
            &format!(
                "{:.1} cuts/min · {:.0} wpm · silence under {:.1}s",
                dna.pacing.cuts_per_minute, dna.pacing.words_per_minute, dna.pacing.max_silence_sec
            ),
        );
        property_row(
            ui,
            "Audio",
            &format!(
                "{:.1} sfx/min · {}",
                dna.audio.sfx_per_minute,
                dna.audio.signature_effects.join(", ")
            ),
        );
        property_row(
            ui,
            "Visual",
            &format!(
                "{:.0}% coverage · longest hold {:.1}s · {} ending",
                dna.visual.broll_coverage * 100.0,
                dna.visual.longest_static_hold_sec,
                dna.visual.ending_style
            ),
        );

        if !dna.transferable_rules.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new("Rules that transfer").small().strong().color(TEXT_PRIMARY));
            for rule in &dna.transferable_rules {
                ui.label(RichText::new(format!("· {rule}")).small().color(TEXT_SECONDARY));
            }
        }
    });
}

fn compare_row(
    ui: &mut Ui,
    state: &mut InsightsState,
    project_open: bool,
    selected: &str,
    outcome: &mut InsightsOutcome,
) {
    panel(ui, |ui| {
        ui.label(
            RichText::new("Presenter reference (optional)")
                .small()
                .color(TEXT_SECONDARY),
        );
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.set_max_width(300.0);
                pro_text_input(ui, &mut state.presenter_reference, "presenter_ref.png");
            });
            ui.label(
                RichText::new(
                    "Needed before a generated shot may show your face; without it those shots \
                     are re-framed to exclude it.",
                )
                .small()
                .color(TEXT_DISABLED),
            );
        });

        ui.add_space(8.0);
        ui.checkbox(
            &mut state.use_llm_director,
            RichText::new("Use the AI director (3-agent pipeline)").small(),
        )
        .on_hover_text(
            "Deconstruct → direct → write prompts, each a strict JSON-schema call.
             Costs tokens and needs an LLM key. Unchecked runs the offline rule engine.",
        );

        ui.add_space(8.0);
        ui.add_enabled_ui(!state.is_busy() && project_open, |ui| {
            if pro_button(ui, "Compare with Current Studio & Generate Edit Plan", true).clicked() {
                outcome.compare_with_studio = Some(selected.to_string());
            }
        });
        if !project_open {
            ui.label(
                RichText::new("Open a project first — there is no timeline to compare against.")
                    .small()
                    .color(WARN),
            );
        }
    });
}

fn pacing(ui: &mut Ui, heatmap: &PacingHeatmap) {
    section_header(ui, "Pacing");
    panel(ui, |ui| {
        // The windows have always been measured; nothing drew them, so the
        // "heatmap" was a list of averages.
        heatmap_chart(ui, heatmap);
        ui.add_space(10.0);
        property_row(ui, "Hook (first 10s)", &format!("{:.0} wpm", heatmap.hook_retention_wpm));
        property_row(ui, "Overall", &format!("{:.0} wpm", heatmap.overall_wpm));
        property_row(ui, "Cuts", &format!("{:.1} per minute", heatmap.jump_cut_frequency));
        property_row(ui, "Mean pause", &format!("{:.2}s", heatmap.mean_gap_sec));
        property_row(ui, "Longest pause", &format!("{:.2}s", heatmap.max_gap_sec));
        property_row(
            ui,
            "Hook shape",
            if heatmap.hook_is_front_loaded() {
                "front-loaded — the opening outruns the body"
            } else {
                "even — the opening matches the body"
            },
        );
    });
}

/// `1234567` → `1,234,567`.
fn thousands(value: f64) -> String {
    let rounded = value.max(0.0).round() as u64;
    let digits = rounded.to_string();

    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_tooling::competitor::models::*;

    #[test]
    fn view_counts_are_grouped_for_reading() {
        assert_eq!(thousands(1_234_567.0), "1,234,567");
        assert_eq!(thousands(999.0), "999");
        assert_eq!(thousands(1_000.0), "1,000");
        assert_eq!(thousands(0.0), "0");
    }

    #[test]
    fn an_empty_channel_id_is_refused_before_any_request() {
        let mut state = InsightsState::default();
        state.analyze_channel();

        assert!(state.error.as_deref().is_some_and(|e| e.contains("channel id")));
        assert!(!state.is_busy(), "nothing was queued");
    }

    #[test]
    fn comparing_an_undeconstructed_video_says_so_rather_than_queueing() {
        let mut state = InsightsState::default();
        state.compare("never_analysed");

        assert!(state.error.as_deref().is_some_and(|e| e.contains("Deconstruct")));
        assert!(!state.is_busy());
        assert!(state.outbox.is_empty(), "nothing was queued for the worker");
    }

    #[test]
    fn the_preflight_is_asked_for_once_not_every_frame() {
        let mut state = InsightsState::default();

        state.request_preflight();
        state.request_preflight();
        state.request_preflight();

        let queued = state.take_requests();
        assert_eq!(queued.len(), 1, "one probe, not one per frame");
        assert!(matches!(queued[0], InsightsRequest::CheckPrerequisites));
    }

    #[test]
    fn rechecking_after_editing_keys_queues_a_fresh_probe() {
        let mut state = InsightsState::default();
        state.request_preflight();
        let _ = state.take_requests();

        state.recheck();
        assert_eq!(state.take_requests().len(), 1);
    }

    #[test]
    fn the_preflight_result_is_absorbed_from_the_worker() {
        let mut state = InsightsState::default();
        assert!(state.prerequisites.is_none(), "unknown until it comes back");

        state.apply_event(&AppEvent::PrerequisitesChecked {
            job: 1,
            report: Prerequisites {
                youtube_key: true,
                llm_key: false,
                yt_dlp: false,
                config_error: None,
            },
        });

        let report = state.prerequisites.as_ref().expect("absorbed");
        assert!(report.can_analyze());
        assert_eq!(report.blockers().len(), 2);
    }

    #[test]
    fn colour_blending_stays_inside_its_endpoints() {
        let a = egui::Color32::from_rgb(0, 0, 0);
        let b = egui::Color32::from_rgb(200, 100, 50);

        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
        // Out of range must not wrap a u8 round the houses.
        assert_eq!(lerp_color(a, b, 5.0), b);
        assert_eq!(lerp_color(a, b, -3.0), a);
    }

    #[test]
    fn a_request_is_queued_for_the_worker_rather_than_run_on_the_render_thread() {
        let mut state = InsightsState::default();
        state.channel_id = "UCabc".into();
        state.analyze_channel();

        assert!(state.is_busy(), "the spinner starts immediately");
        let queued = state.take_requests();
        assert_eq!(queued.len(), 1);
        assert!(matches!(queued[0], InsightsRequest::AnalyzeChannel(ref id) if id == "UCabc"));
        assert!(state.take_requests().is_empty(), "draining is destructive");
    }

    #[test]
    fn a_terminal_event_clears_the_spinner_whatever_happened_before_it() {
        let mut state = InsightsState::default();
        state.channel_id = "UCabc".into();
        state.analyze_channel();
        let _ = state.take_requests();

        state.apply_event(&AppEvent::AnalysisProgress {
            job: 1,
            fraction: 0.5,
            stage: "sampling".into(),
        });
        assert_eq!(state.progress.as_ref().map(|p| p.0), Some(0.5));

        state.apply_event(&AppEvent::Error {
            job: Some(1),
            message: "no key".into(),
        });
        assert!(state.is_busy(), "an error is not the end of the job");

        state.apply_event(&AppEvent::Finished { job: 1 });
        assert!(!state.is_busy());
        assert!(state.progress.is_none(), "the bar goes with the spinner");
        assert_eq!(state.error.as_deref(), Some("no key"));
    }

    #[test]
    fn a_finished_plan_is_handed_back_rather_than_applied_here() {
        let mut state = InsightsState::default();
        let plan = RevisionPlan {
            competitor_video_id: "v".into(),
            competitor_title: "t".into(),
            tasks: Vec::new(),
            summary: vec!["cut faster".into()],
        };

        let handed_back = state.apply_event(&AppEvent::RevisionsReady {
            job: 1,
            plan: Box::new(plan.clone()),
        });
        assert_eq!(handed_back.map(|p| p.competitor_title), Some("t".into()));
    }

    #[test]
    fn a_missing_duration_falls_back_rather_than_analysing_a_zero_length_video() {
        let mut state = InsightsState::default();
        assert_eq!(state.duration_of("unknown"), ASSUMED_DURATION_SEC);

        state.durations.insert("known".into(), 612.0);
        assert_eq!(state.duration_of("known"), 612.0);
    }

    #[test]
    fn the_shot_mix_counts_tags_most_common_first() {
        let segment = |visual| TranscriptSegment {
            start_sec: 0.0,
            end_sec: 1.0,
            text: String::new(),
            visual,
        };
        let video = CompetitorVideo {
            video_id: "a".into(),
            channel_id: "UC1".into(),
            title: "t".into(),
            view_count: 0,
            outlier_multiplier: 1.0,
            duration_sec: 60.0,
            published_at: None,
            retention: RetentionAnalysis::default(),
            audio: AudioDynamics::default(),
            structure: VisualAndPacingStructure::default(),
            transcript: TranscriptAndHooks {
                segments: vec![
                    segment(VlmTag::TalkingHead),
                    segment(VlmTag::BRoll),
                    segment(VlmTag::TalkingHead),
                ],
                ..Default::default()
            },
            analyzed_at: 0,
        };

        assert_eq!(shot_mix(&video), "talking head ×2, b-roll ×1");
    }
}
