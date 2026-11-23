// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! GUI entry point: builds the shared session state (settings,
//! terminal, font, clipboard, event proxy) and drives the winit
//! event loop until the window closes.

use winit::event_loop::EventLoop;
use winit::keyboard::ModifiersState;

use crate::font::load_default_face;
use crate::settings::SettingsManager;
use crate::vt::Terminal;

use super::app::App;

/// Entry point of the winit GUI (called from `main` with no args).
pub fn run() {
    let settings_mgr = SettingsManager::load_default_location();
    let settings = settings_mgr.get().clone();

    let term = Terminal::new(80, 24);
    let palette = App::palette_from_settings(&settings);
    let face = match load_default_face(&settings.font, settings.font_size as f32) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };

    let event_loop = EventLoop::with_user_event()
        .build()
        .expect("build winit event loop");
    let proxy = event_loop.create_proxy();

    let glass = App::glass_from_settings(&settings);
    // One clipboard owner for the whole session (see the field docs:
    // a per-copy instance loses the X11 selection ownership on drop).
    let clipboard = match arboard::Clipboard::new() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("clipboard unavailable, copy/paste disabled: {e}");
            None
        }
    };
    let mut app = App {
        settings,
        settings_mgr,
        term,
        pty: None,
        face,
        palette,
        glass,
        scroll_offset: 0,
        mods: ModifiersState::empty(),
        window: None,
        gl: None,
        x11_window_id: None,
        selection: None,
        select_anchor: None,
        mouse_pos: (0, 0),
        proxy,
        clipboard,
    };

    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop error: {e}");
    }
}
