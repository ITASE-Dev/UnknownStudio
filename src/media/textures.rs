//! RGB frames → GPU textures, keyed by a caller-chosen string (a source path
//! for thumbnails, a fixed key for the program monitor). Uploading in place
//! reuses the allocation instead of churning a texture per frame.

use crate::media::decoder::RgbFrame;
use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions};
use std::collections::HashMap;

/// A texture plus the source aspect (w/h) it must be drawn at.
#[derive(Clone, Copy)]
pub struct Poster {
    pub id: TextureId,
    pub aspect: f32,
}

#[derive(Default)]
pub struct Textures {
    entries: HashMap<String, (TextureHandle, f32)>,
}

impl Textures {
    pub fn set(&mut self, ctx: &egui::Context, key: impl Into<String>, frame: &RgbFrame) {
        let key = key.into();
        let image = ColorImage::from_rgb([frame.width as usize, frame.height as usize], &frame.rgb);
        let aspect = if frame.height > 0 {
            frame.width as f32 / frame.height as f32
        } else {
            16.0 / 9.0
        };

        match self.entries.get_mut(&key) {
            Some((handle, stored)) => {
                handle.set(image, TextureOptions::LINEAR);
                *stored = aspect;
            }
            None => {
                let handle = ctx.load_texture(key.clone(), image, TextureOptions::LINEAR);
                self.entries.insert(key, (handle, aspect));
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<Poster> {
        self.entries.get(key).map(|(handle, aspect)| Poster {
            id: handle.id(),
            aspect: *aspect,
        })
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }
}
