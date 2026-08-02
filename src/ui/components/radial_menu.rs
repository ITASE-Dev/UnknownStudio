//! Pie menu: actions laid out on a circle around the click point.
//!
//! egui has no radial menu, so this is painted by hand. Layout and hit-testing
//! both work in polar coordinates around the centre — the angle picks the slice,
//! the distance decides whether anything is picked at all.

use crate::models::selection::MediaSelection;
use crate::ui::core::icons;
use crate::ui::theme::tokens::*;
use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Key, Order, Pos2, Rect, Sense, Shape, Stroke, Vec2,
};
use std::f32::consts::TAU;

/// Distance from the centre to each button's centre.
const RADIUS: f32 = 78.0;
/// Inside this radius nothing is selected — the rest position, and the way to
/// dismiss the menu without picking anything.
const DEAD_ZONE: f32 = 26.0;
/// Past this the pointer has left the menu entirely.
const OUTER_LIMIT: f32 = RADIUS + 34.0;
const BUTTON_RADIUS: f32 = 26.0;
/// Slices are drawn from this many straight segments.
const ARC_SEGMENTS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadialAction {
    /// Hand the selection to the director as chat context.
    SendToAiChat,
    /// Cut the selection at the playhead.
    Split,
    Properties,
    Delete,
    Cancel,
}

impl RadialAction {
    pub fn icon(self) -> &'static str {
        match self {
            Self::SendToAiChat => icons::SEND_TO_CHAT,
            Self::Split => icons::TRIM,
            Self::Properties => icons::PROPERTIES,
            Self::Delete => icons::DELETE,
            Self::Cancel => icons::CANCEL,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SendToAiChat => "Ask AI",
            Self::Split => "Split",
            Self::Properties => "Info",
            Self::Delete => "Delete",
            Self::Cancel => "Cancel",
        }
    }

    pub fn is_destructive(self) -> bool {
        matches!(self, Self::Delete)
    }

    /// Actions that make sense for what was clicked. Order is the ring order,
    /// starting at the top and going clockwise.
    pub fn for_selection(selection: &MediaSelection) -> Vec<Self> {
        let mut actions = vec![Self::SendToAiChat, Self::Properties];
        if selection.is_splittable() {
            actions.insert(1, Self::Split);
        }
        if selection.is_removable() {
            actions.push(Self::Delete);
        }
        actions.push(Self::Cancel);
        actions
    }
}

/// Gate on the press/release cycle.
///
/// The menu opens on a *click*, and egui reports a click on the button's
/// release — so the frame the menu first paints is already a release frame.
/// Committing on that release would pick nothing and close the menu again,
/// which is why a release only counts once a press has been seen while open.
#[derive(Default)]
struct Commit {
    /// The frame the menu opened on, whose input belongs to the click that
    /// asked for it. A fast click lands its press and release in that one
    /// frame, so arming alone is not enough to ignore it.
    fresh: bool,
    armed: bool,
}

impl Commit {
    fn reset(&mut self) {
        self.fresh = true;
        self.armed = false;
    }

    /// Whether this frame's input should act on the highlighted slice.
    fn poll(&mut self, pressed: bool, released: bool) -> bool {
        if self.fresh {
            self.fresh = false;
            return false;
        }
        self.armed |= pressed;
        self.armed && released
    }
}

#[derive(Default)]
pub struct RadialMenu {
    center: Pos2,
    target: Option<MediaSelection>,
    actions: Vec<RadialAction>,
    /// Index under the pointer, recomputed every frame.
    hovered: Option<usize>,
    commit: Commit,
}

impl RadialMenu {
    /// Opens the menu at `pos` for `selection`, replacing any open one.
    pub fn open_at(&mut self, pos: Pos2, selection: MediaSelection) {
        self.actions = RadialAction::for_selection(&selection);
        self.target = Some(selection);
        self.center = pos;
        self.hovered = None;
        self.commit.reset();
    }

    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    pub fn target(&self) -> Option<&MediaSelection> {
        self.target.as_ref()
    }

    /// Takes the selection the menu was opened for, closing it.
    pub fn take_target(&mut self) -> Option<MediaSelection> {
        self.hovered = None;
        self.actions.clear();
        self.commit.reset();
        self.target.take()
    }

    pub fn close(&mut self) {
        let _ = self.take_target();
    }

    /// Draws the menu and reports the chosen action. `Cancel` (and Escape, and
    /// a click outside) closes it and yields nothing to act on.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<RadialAction> {
        if !self.is_open() {
            return None;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.close();
            return None;
        }

        let size = Vec2::splat(OUTER_LIMIT * 2.0);
        let area = egui::Area::new(egui::Id::new("radial_menu"))
            .order(Order::Foreground)
            .fixed_pos(self.center - size / 2.0)
            .interactable(true);

        let mut chosen = None;

        area.show(ctx, |ui| {
            // Allocated so the menu, not the panel underneath, owns these pixels.
            let (rect, _response) = ui.allocate_exact_size(size, Sense::click());
            let center = rect.center();
            let pointer = ui.ctx().pointer_latest_pos();

            self.hovered = pointer.and_then(|pos| self.slice_at(center, pos));
            self.paint(ui.painter(), center);

            // Either button commits, so a menu opened by the right button can be
            // picked from with the left one.
            let (pressed, released) =
                ui.input(|i| (i.pointer.any_pressed(), i.pointer.any_released()));
            if self.commit.poll(pressed, released) {
                chosen = match self.hovered {
                    Some(index) => self.actions.get(index).copied(),
                    // Released in the dead zone or outside: dismissed.
                    None => Some(RadialAction::Cancel),
                };
            }
        });

        match chosen {
            Some(RadialAction::Cancel) => {
                self.close();
                None
            }
            action @ Some(_) => action,
            None => None,
        }
    }

    /// Which slice the pointer is over, in polar terms.
    fn slice_at(&self, center: Pos2, pointer: Pos2) -> Option<usize> {
        let offset = pointer - center;
        let distance = offset.length();
        if distance < DEAD_ZONE || distance > OUTER_LIMIT || self.actions.is_empty() {
            return None;
        }

        let slice = TAU / self.actions.len() as f32;
        // Screen y grows downward, and the ring starts at the top: rotate by a
        // quarter turn so index 0 sits at 12 o'clock.
        let angle = (offset.y.atan2(offset.x) + TAU / 4.0 + slice / 2.0).rem_euclid(TAU);
        Some((angle / slice) as usize % self.actions.len())
    }

    /// Angle of a button's centre, measured the same way as `slice_at`.
    fn angle_of(&self, index: usize) -> f32 {
        let slice = TAU / self.actions.len() as f32;
        index as f32 * slice - TAU / 4.0
    }

    fn button_center(&self, center: Pos2, index: usize) -> Pos2 {
        let angle = self.angle_of(index);
        center + Vec2::new(angle.cos(), angle.sin()) * RADIUS
    }

    fn paint(&self, painter: &egui::Painter, center: Pos2) {
        let count = self.actions.len();
        if count == 0 {
            return;
        }
        let slice = TAU / count as f32;

        // Backdrop ring, so the menu reads against busy footage.
        painter.circle_filled(center, OUTER_LIMIT, BG_APP.gamma_multiply(0.82));
        painter.circle_stroke(center, OUTER_LIMIT, Stroke::new(1.0_f32, BORDER));

        for (index, action) in self.actions.iter().enumerate() {
            let hovered = self.hovered == Some(index);
            if hovered {
                let mid = self.angle_of(index);
                painter.add(annulus_slice(
                    center,
                    DEAD_ZONE,
                    OUTER_LIMIT,
                    mid - slice / 2.0,
                    mid + slice / 2.0,
                    accent_for(*action).gamma_multiply(0.22),
                ));
            }

            let button = self.button_center(center, index);
            let (fill, tint) = match (hovered, action.is_destructive()) {
                (true, true) => (ERR.gamma_multiply(0.85), Color32::WHITE),
                (true, false) => (ACCENT.gamma_multiply(0.85), Color32::WHITE),
                (false, true) => (BG_ELEVATED, ERR),
                (false, false) => (BG_ELEVATED, TEXT_SECONDARY),
            };

            painter.circle(
                button,
                BUTTON_RADIUS,
                fill,
                Stroke::new(1.0_f32, if hovered { tint } else { BORDER_STRONG }),
            );
            painter.text(
                button,
                Align2::CENTER_CENTER,
                action.icon(),
                FontId::new(18.0, FontFamily::Proportional),
                tint,
            );

            if hovered {
                // Label sits further out along the same spoke, so it never
                // covers a neighbouring button.
                let angle = self.angle_of(index);
                let label_at = center + Vec2::new(angle.cos(), angle.sin()) * (OUTER_LIMIT + 16.0);
                painter.text(
                    label_at,
                    Align2::CENTER_CENTER,
                    action.label(),
                    FontId::new(11.0, FontFamily::Proportional),
                    TEXT_PRIMARY,
                );
            }
        }

        // Hub: the title of what was right-clicked, elided to the dead zone.
        painter.circle_filled(center, DEAD_ZONE, BG_PANEL);
        painter.circle_stroke(center, DEAD_ZONE, Stroke::new(1.0_f32, BORDER));
        if let Some(target) = &self.target {
            painter.text(
                center,
                Align2::CENTER_CENTER,
                truncate(&target.title(), 6),
                FontId::new(9.0, FontFamily::Monospace),
                TEXT_DISABLED,
            );
        }
    }
}

fn accent_for(action: RadialAction) -> Color32 {
    if action.is_destructive() {
        ERR
    } else {
        ACCENT
    }
}

/// Filled ring segment between two angles.
fn annulus_slice(
    center: Pos2,
    inner: f32,
    outer: f32,
    from: f32,
    to: f32,
    fill: Color32,
) -> Shape {
    let mut points = Vec::with_capacity((ARC_SEGMENTS + 1) * 2);
    let step = (to - from) / ARC_SEGMENTS as f32;

    for i in 0..=ARC_SEGMENTS {
        let angle = from + step * i as f32;
        points.push(center + Vec2::new(angle.cos(), angle.sin()) * outer);
    }
    for i in (0..=ARC_SEGMENTS).rev() {
        let angle = from + step * i as f32;
        points.push(center + Vec2::new(angle.cos(), angle.sin()) * inner);
    }

    Shape::convex_polygon(points, fill, Stroke::NONE)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…"
}

/// Screen rect the menu occupies — for callers that need to avoid overlapping it.
pub fn menu_bounds(center: Pos2) -> Rect {
    Rect::from_center_size(center, Vec2::splat(OUTER_LIMIT * 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(actions: Vec<RadialAction>) -> RadialMenu {
        let mut menu = RadialMenu::default();
        menu.center = Pos2::ZERO;
        menu.target = Some(MediaSelection::PreviewScreen { seconds: 0.0 });
        menu.actions = actions;
        menu.commit.reset();
        menu
    }

    /// One frame of pointer input, as `show` reads it.
    fn frame(commit: &mut Commit, pressed: bool, released: bool) -> bool {
        commit.poll(pressed, released)
    }

    fn four() -> RadialMenu {
        menu(vec![
            RadialAction::SendToAiChat,
            RadialAction::Split,
            RadialAction::Properties,
            RadialAction::Cancel,
        ])
    }

    #[test]
    fn index_zero_sits_at_twelve_oclock() {
        let menu = four();
        let center = Pos2::new(100.0, 100.0);

        // Screen y grows downward, so "up" is negative y.
        let up = Pos2::new(center.x, center.y - RADIUS);
        let right = Pos2::new(center.x + RADIUS, center.y);
        let down = Pos2::new(center.x, center.y + RADIUS);
        let left = Pos2::new(center.x - RADIUS, center.y);

        assert_eq!(menu.slice_at(center, up), Some(0));
        assert_eq!(menu.slice_at(center, right), Some(1));
        assert_eq!(menu.slice_at(center, down), Some(2));
        assert_eq!(menu.slice_at(center, left), Some(3));
    }

    #[test]
    fn button_centers_match_the_slices_they_belong_to() {
        let menu = four();
        let center = Pos2::new(40.0, 40.0);

        for index in 0..menu.actions.len() {
            let button = menu.button_center(center, index);
            assert_eq!(menu.slice_at(center, button), Some(index));
        }
    }

    #[test]
    fn the_dead_zone_and_the_outside_select_nothing() {
        let menu = four();
        let center = Pos2::new(0.0, 0.0);

        assert_eq!(menu.slice_at(center, center), None);
        assert_eq!(menu.slice_at(center, Pos2::new(0.0, -DEAD_ZONE + 1.0)), None);
        assert_eq!(menu.slice_at(center, Pos2::new(0.0, -OUTER_LIMIT - 1.0)), None);
    }

    #[test]
    fn slices_tile_the_full_circle_without_gaps() {
        let menu = four();
        let center = Pos2::new(0.0, 0.0);
        let mut seen = [false; 4];

        for step in 0..360 {
            let angle = step as f32 * TAU / 360.0;
            let probe = center + Vec2::new(angle.cos(), angle.sin()) * RADIUS;
            let index = menu.slice_at(center, probe).expect("every angle hits a slice");
            seen[index] = true;
        }
        assert!(seen.iter().all(|hit| *hit), "every slice is reachable");
    }

    #[test]
    fn the_action_set_follows_what_was_clicked() {
        let clip = MediaSelection::Clip {
            id: 1,
            label: "a".into(),
            track: "V1".into(),
            start_seconds: 0.0,
            duration_seconds: 1.0,
        };
        let actions = RadialAction::for_selection(&clip);
        assert!(actions.contains(&RadialAction::Delete));
        assert!(actions.contains(&RadialAction::Split));
        assert_eq!(actions.first(), Some(&RadialAction::SendToAiChat));

        let preview = MediaSelection::PreviewScreen { seconds: 0.0 };
        let actions = RadialAction::for_selection(&preview);
        assert!(!actions.contains(&RadialAction::Delete));
        assert!(!actions.contains(&RadialAction::Split));
        assert!(actions.contains(&RadialAction::Cancel));
    }

    #[test]
    fn the_click_that_opens_the_menu_does_not_also_close_it() {
        let mut commit = Commit::default();
        commit.reset();

        // The menu paints for the first time on the frame the opening click was
        // released — the frame that used to dismiss it immediately.
        assert!(!frame(&mut commit, false, true), "opening release is not a choice");
        assert!(!frame(&mut commit, false, false), "menu stays open while idle");
        assert!(!frame(&mut commit, false, false));
    }

    #[test]
    fn a_fast_click_lands_press_and_release_in_the_opening_frame() {
        let mut commit = Commit::default();
        commit.reset();

        // At 30fps a quick click reports both in one frame; it still belongs to
        // the click that opened the menu.
        assert!(!frame(&mut commit, true, true));
        assert!(!frame(&mut commit, false, false));
    }

    #[test]
    fn the_next_click_commits_on_its_release() {
        let mut commit = Commit::default();
        commit.reset();
        frame(&mut commit, false, true);

        assert!(!frame(&mut commit, true, false), "pressing only arms");
        assert!(!frame(&mut commit, false, false), "held, still choosing");
        assert!(frame(&mut commit, false, true), "release picks the slice");
    }

    #[test]
    fn a_stray_release_without_a_press_is_ignored() {
        let mut commit = Commit::default();
        commit.reset();
        frame(&mut commit, false, false);

        // Releasing a button that went down before the menu opened — a drag
        // ending, say — must not count as a choice.
        assert!(!frame(&mut commit, false, true));
    }

    #[test]
    fn reopening_the_menu_starts_the_cycle_again() {
        let mut menu = four();
        menu.commit.armed = true;
        menu.open_at(Pos2::new(5.0, 5.0), MediaSelection::PreviewScreen { seconds: 0.0 });

        assert!(!frame(&mut menu.commit, false, true), "the reopening click");
    }

    #[test]
    fn taking_the_target_closes_the_menu() {
        let mut menu = four();
        assert!(menu.is_open());
        assert!(menu.take_target().is_some());
        assert!(!menu.is_open());
        assert!(menu.take_target().is_none());
    }
}
