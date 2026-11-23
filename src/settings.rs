// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! Port of `settings.py`: validated, atomically-saved settings stored in
//! `~/.config/tuxixi/config.json` (the path is injectable for tests).
//!
//! The GObject signal `settings-changed` is replaced by listener
//! callbacks registered with [`SettingsManager::on_change`]. Each
//! listener receives the NEW settings snapshot, so it never needs to
//! re-borrow the manager (which would deadlock the GUI call sites that
//! mutate settings through `RefCell::borrow_mut`).

mod blur_mode;
mod manager;
mod model;

pub use blur_mode::BlurMode;
pub use manager::{ChangeListener, SettingsManager};
pub use model::{
    is_valid_blur_mode, is_valid_theme, Settings, DEFAULT_FONT, DEFAULT_FONT_SIZE,
    DEFAULT_GLASS_OPACITY, LEGACY_DEFAULT_FONT_SIZE, MAX_FONT_SIZE, MAX_GLASS_OPACITY,
    MIN_FONT_SIZE, MIN_GLASS_OPACITY,
};

#[cfg(test)]
mod tests;
