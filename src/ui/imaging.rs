//! Aspect-correct image fitting. Nothing in the UI stretches pixels: video is
//! either letterboxed (`contain`) or cropped (`cover`).

use eframe::egui::{Pos2, Rect, Vec2};

/// Full UV span — the default source rect for an untouched texture.
pub fn full_uv() -> Rect {
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
}

/// Largest rect with `aspect` (w/h) that fits inside `outer`, centered.
/// Leaves letterbox bars — used where the whole frame must stay visible.
pub fn contain_rect(outer: Rect, aspect: f32) -> Rect {
    if aspect <= 0.0 || outer.width() <= 0.0 || outer.height() <= 0.0 {
        return outer;
    }
    let outer_aspect = outer.width() / outer.height();
    let size = if aspect > outer_aspect {
        Vec2::new(outer.width(), outer.width() / aspect)
    } else {
        Vec2::new(outer.height() * aspect, outer.height())
    };
    Rect::from_center_size(outer.center(), size)
}

/// UV rect that crops a texture of `aspect` to fill a box of `target_aspect`
/// without distortion — the thumbnail equivalent of `object-fit: cover`.
pub fn cover_uv(target_aspect: f32, aspect: f32) -> Rect {
    if aspect <= 0.0 || target_aspect <= 0.0 {
        return full_uv();
    }
    if aspect > target_aspect {
        // Source is wider: trim the sides.
        let keep = target_aspect / aspect;
        let margin = (1.0 - keep) / 2.0;
        Rect::from_min_max(Pos2::new(margin, 0.0), Pos2::new(1.0 - margin, 1.0))
    } else {
        // Source is taller: trim top and bottom.
        let keep = aspect / target_aspect;
        let margin = (1.0 - keep) / 2.0;
        Rect::from_min_max(Pos2::new(0.0, margin), Pos2::new(1.0, 1.0 - margin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(160.0, 90.0))
    }

    #[test]
    fn contain_letterboxes_a_taller_source() {
        let fitted = contain_rect(plate(), 1.0);
        assert_eq!(fitted.height(), 90.0);
        assert_eq!(fitted.width(), 90.0);
        assert_eq!(fitted.center(), plate().center());
    }

    #[test]
    fn contain_pillarboxes_a_wider_source() {
        let fitted = contain_rect(plate(), 4.0);
        assert_eq!(fitted.width(), 160.0);
        assert_eq!(fitted.height(), 40.0);
    }

    #[test]
    fn matching_aspect_fills_exactly() {
        let fitted = contain_rect(plate(), 16.0 / 9.0);
        assert!((fitted.width() - 160.0).abs() < 0.01);
        assert_eq!(cover_uv(16.0 / 9.0, 16.0 / 9.0), full_uv());
    }

    #[test]
    fn cover_trims_the_long_axis_only() {
        let wide = cover_uv(1.0, 2.0);
        assert_eq!((wide.min.x, wide.max.x), (0.25, 0.75));
        assert_eq!((wide.min.y, wide.max.y), (0.0, 1.0));

        let tall = cover_uv(2.0, 1.0);
        assert_eq!((tall.min.y, tall.max.y), (0.25, 0.75));
        assert_eq!((tall.min.x, tall.max.x), (0.0, 1.0));
    }
}
