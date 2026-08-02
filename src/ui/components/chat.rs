use crate::ui::core::buttons::{icon_button_painted, pro_button, Icon};
use crate::ui::core::inputs::pro_text_area;
use crate::ui::responsive::breakpoint;
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align, Layout, Margin, Pos2, Response, RichText, Rounding, Sense, Stroke, Ui, Vec2,
};

/// Namespaced entry points for the chat surface. Each delegates to the free
/// function below it, so the components stay usable either way.
pub struct AiChatBubble;
pub struct UserChatBubble;
pub struct NoticeBubble;
pub struct PromptInputArea;

impl AiChatBubble {
    pub const AUTHOR: &'static str = "Director";

    pub fn show(ui: &mut Ui, text: &str) -> Response {
        ai_chat_bubble(ui, Self::AUTHOR, text)
    }

    pub fn show_as(ui: &mut Ui, author: &str, text: &str) -> Response {
        ai_chat_bubble(ui, author, text)
    }
}

impl UserChatBubble {
    pub fn show(ui: &mut Ui, text: &str) -> Response {
        user_chat_bubble(ui, text)
    }
}

impl NoticeBubble {
    /// Out-of-band message (a failure, a status note). Deliberately unlike the
    /// assistant bubble: it is not part of the conversation the model sees.
    pub fn show(ui: &mut Ui, text: &str) -> Response {
        notice_bubble(ui, text)
    }
}

impl PromptInputArea {
    /// Returns the submitted prompt, clearing the field.
    pub fn show(ui: &mut Ui, text: &mut String, enabled: bool) -> Option<String> {
        let pending = text.trim().to_owned();
        let submitted = ui
            .add_enabled_ui(enabled, |ui| prompt_input_area(ui, text))
            .inner;
        (submitted && !pending.is_empty()).then_some(pending)
    }
}

/// Bubble measure: most of the row, capped so long lines stay readable.
pub fn bubble_width(ui: &Ui) -> f32 {
    let avail = ui.available_width();
    (avail * if breakpoint(ui).is_compact() { 0.96 } else { 0.82 }).clamp(120.0, 620.0).min(avail)
}

/// Narrowest a bubble may be before it stops being readable.
const MIN_BUBBLE_W: f32 = 80.0;

/// Places one bubble in its own row, at a width it can rely on.
///
/// Alignment is an **offset**, not a layout direction. Right-aligning with
/// `Layout::right_to_left` looks equivalent and is not: the frame's content
/// inherits that direction, and a wrapping label inside a right-to-left `Ui`
/// measures its wrap width from the opposite edge. Short messages hugged the
/// right correctly while long ones resolved to nearly the full panel width, so
/// the column appeared to jump sideways depending on message length.
///
/// Here the row is a plain left-to-right `horizontal` and the layout stays
/// top-down, so text always wraps against `width` — the same measurement for
/// every message, long or short.
///
/// Two things do the aligning, and both are needed:
///
/// - the leading `add_space` puts the *region* against the right edge, and
/// - `Align::Max` puts the *bubble* against the right edge of that region.
///
/// Without the second, a short message hugs the left of a full-measure region
/// and lands in the middle of the panel while a long one reaches the edge —
/// which is the jump this function exists to prevent.
fn bubble_row(
    ui: &mut Ui,
    align_right: bool,
    max_width: f32,
    add: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    let available = ui.available_width().max(MIN_BUBBLE_W);
    let width = max_width.clamp(MIN_BUBBLE_W, available);
    let cross = if align_right { Align::Max } else { Align::Min };

    ui.horizontal(|ui| {
        if align_right {
            ui.add_space((available - width).max(0.0));
        }
        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(cross), |ui| {
            // The frame subtracts its own margins from this, so the label
            // wraps at exactly the bubble's inner width.
            ui.set_max_width(width);
            add(ui)
        })
        .inner
    })
    .inner
}

/// Left-aligned assistant bubble: elevated plate, accent hairline, squared top-left.
pub fn ai_chat_bubble(ui: &mut Ui, author: &str, body: &str) -> Response {
    let w = bubble_width(ui);
    ai_chat_bubble_sized(ui, author, body, w)
}

pub fn ai_chat_bubble_sized(ui: &mut Ui, author: &str, body: &str, max_width: f32) -> Response {
    bubble_row(ui, false, max_width, |ui| {
        egui::Frame::none()
            .fill(BG_ELEVATED)
            .stroke(Stroke::new(1.0_f32, ACCENT.linear_multiply(0.35)))
            .rounding(Rounding { nw: 2.0, ne: 6.0, sw: 6.0, se: 6.0 })
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.label(RichText::new(author).small().strong().color(ACCENT));
                ui.add_space(2.0);
                ui.label(RichText::new(body).color(TEXT_PRIMARY));
            })
            .response
    })
}

/// Right-aligned user bubble: sunken plate, squared top-right.
pub fn user_chat_bubble(ui: &mut Ui, body: &str) -> Response {
    let w = bubble_width(ui);
    user_chat_bubble_sized(ui, body, w)
}

pub fn user_chat_bubble_sized(ui: &mut Ui, body: &str, max_width: f32) -> Response {
    bubble_row(ui, true, max_width, |ui| {
        egui::Frame::none()
            .fill(BG_SUNKEN)
            .stroke(hairline())
            .rounding(Rounding { nw: 6.0, ne: 2.0, sw: 6.0, se: 6.0 })
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.label(RichText::new(body).color(TEXT_SECONDARY));
            })
            .response
    })
}

/// Full-width notice plate for failures and system messages.
pub fn notice_bubble(ui: &mut Ui, body: &str) -> Response {
    egui::Frame::none()
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0_f32, ERR.linear_multiply(0.55)))
        .rounding(R_SM)
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width());
            ui.label(RichText::new(body).small().color(ERR));
        })
        .response
}

/// Three-dot "thinking" indicator; `time` drives the phase offsets.
pub fn typing_indicator(ui: &mut Ui, time: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(58.0, 26.0), Sense::hover());
    let p = ui.painter();
    p.rect(rect, R, BG_ELEVATED, hairline());
    for i in 0..3 {
        let phase = time * 4.0 - i as f32 * 0.5;
        let a = 0.35 + 0.65 * (phase.sin() * 0.5 + 0.5);
        p.circle_filled(
            Pos2::new(rect.left() + 16.0 + i as f32 * 13.0, rect.center().y),
            3.0,
            ACCENT.linear_multiply(a),
        );
    }
    resp
}

/// Composer: prompt field + action row. Secondary affordances drop out when the
/// row is too narrow to hold them beside Send.
/// Returns `true` on submit (Send pressed, or ⌘/Ctrl+Enter) and clears `text`.
pub fn prompt_input_area(ui: &mut Ui, text: &mut String) -> bool {
    let mut submitted = false;
    let compact = breakpoint(ui).is_compact();
    egui::Frame::none()
        .fill(BG_PANEL)
        .stroke(hairline())
        .rounding(R)
        .inner_margin(Margin::same(if compact { 6.0 } else { 8.0 }))
        .show(ui, |ui| {
            let field = pro_text_area(
                ui,
                text,
                "Direct the edit… e.g. “tighten the intro to 20s”",
                3,
            );
            let hotkey = field.has_focus()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Enter) && (i.modifiers.command || i.modifiers.ctrl)
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                icon_button_painted(ui, Icon::Plus, true, false).on_hover_text("Attach media");
                if !compact {
                    icon_button_painted(ui, Icon::Mic, true, false).on_hover_text("Dictate");
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(!text.trim().is_empty(), |ui| {
                        if pro_button(ui, "Send", true).clicked() {
                            submitted = true;
                        }
                    });
                    if ui.available_width() > 40.0 {
                        ui.label(RichText::new("⌘↩").small().color(TEXT_DISABLED));
                    }
                });
            });
            if hotkey && !text.trim().is_empty() {
                submitted = true;
            }
        });
    if submitted {
        text.clear();
    }
    submitted
}

/// Scrolling transcript that keeps the composer pinned to the bottom edge.
pub fn chat_column<R>(ui: &mut Ui, composer_h: f32, transcript: impl FnOnce(&mut Ui) -> R) {
    let h = (ui.available_height() - composer_h).max(80.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(h)
        .stick_to_bottom(true)
        .show(ui, transcript);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Context, Rect, RawInput};

    const PANEL_W: f32 = 600.0;

    /// Lays the closure out in a real context and returns what it produced.
    ///
    /// Two frames: egui settles galley sizes on the second, and a bubble's
    /// width depends on its wrapped text.
    fn lay_out<R: Clone>(add: impl Fn(&mut Ui) -> R) -> (R, Rect) {
        let ctx = Context::default();
        let input = || RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(PANEL_W, 800.0),
            )),
            ..Default::default()
        };

        let mut captured: Option<R> = None;
        let mut panel = Rect::NOTHING;
        for _ in 0..2 {
            captured = None;
            ctx.run(input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panel = ui.max_rect();
                    captured = Some(add(ui));
                });
            });
        }
        (captured.expect("laid out"), panel)
    }

    const SHORT: &str = "cut it";
    const LONG: &str = "Tighten the opening to twenty seconds, drop the second B-roll, \
        and put a whoosh on every hard cut in the first minute so the pacing matches \
        the reference video we looked at earlier this afternoon.";

    #[test]
    fn a_user_bubble_is_right_aligned_whatever_its_length() {
        let (short, panel) = lay_out(|ui| user_chat_bubble(ui, SHORT));
        let (long, _) = lay_out(|ui| user_chat_bubble(ui, LONG));

        // Both end at the same place: that edge is what the eye tracks, and
        // it is what used to move.
        assert!(
            (short.rect.right() - long.rect.right()).abs() < 1.0,
            "right edges disagree: short {} vs long {}",
            short.rect.right(),
            long.rect.right()
        );
        assert!(
            short.rect.right() <= panel.right() + 1.0,
            "bubble overflowed the panel"
        );
    }

    #[test]
    fn a_short_user_bubble_does_not_span_the_whole_panel() {
        let (short, panel) = lay_out(|ui| user_chat_bubble(ui, SHORT));

        // The regression looked like this: a bubble ballooning to full width
        // and dragging its text across the panel.
        assert!(
            short.rect.width() < panel.width() * 0.6,
            "a two-word message took {:.0}px of {:.0}px",
            short.rect.width(),
            panel.width()
        );
        assert!(short.rect.left() > panel.left(), "it should be inset from the left");
    }

    #[test]
    fn an_assistant_bubble_is_left_aligned_whatever_its_length() {
        let (short, panel) = lay_out(|ui| AiChatBubble::show(ui, SHORT));
        let (long, _) = lay_out(|ui| AiChatBubble::show(ui, LONG));

        assert!(
            (short.rect.left() - long.rect.left()).abs() < 1.0,
            "left edges disagree: short {} vs long {}",
            short.rect.left(),
            long.rect.left()
        );
        assert!((short.rect.left() - panel.left()).abs() < 1.0, "flush left");
    }

    #[test]
    fn the_two_sides_do_not_meet_in_the_middle() {
        let (user, _) = lay_out(|ui| user_chat_bubble(ui, SHORT));
        let (ai, _) = lay_out(|ui| AiChatBubble::show(ui, SHORT));

        assert!(
            ai.rect.left() < user.rect.left(),
            "the assistant must sit left of the user"
        );
    }

    #[test]
    fn a_long_message_wraps_instead_of_running_off_the_edge() {
        let (long, panel) = lay_out(|ui| user_chat_bubble(ui, LONG));

        assert!(
            long.rect.width() <= panel.width() + 1.0,
            "bubble is {:.0}px wide in a {:.0}px panel",
            long.rect.width(),
            panel.width()
        );
        // Wrapping means height, not width: a single unwrapped line would be
        // one row tall.
        assert!(long.rect.height() > 40.0, "it did not wrap");
    }

    #[test]
    fn a_narrow_panel_still_produces_a_usable_bubble() {
        let ctx = Context::default();
        let mut rect = Rect::NOTHING;
        for _ in 0..2 {
            ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(140.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        rect = user_chat_bubble(ui, LONG).rect;
                    });
                },
            );
        }

        assert!(rect.width() >= MIN_BUBBLE_W - 1.0, "collapsed to {}", rect.width());
        assert!(rect.width() <= 140.0 + 1.0, "overflowed a narrow panel");
    }

    #[test]
    fn the_measure_is_capped_so_a_wide_window_does_not_produce_one_long_line() {
        let ctx = Context::default();
        let mut width = 0.0;
        ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(2400.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    width = bubble_width(ui);
                });
            },
        );

        assert!(width <= 620.0, "{width} exceeds the readable measure");
    }
}
