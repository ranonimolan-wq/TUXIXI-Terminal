// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! The winit `ApplicationHandler` for [`App`]: window lifecycle
//! (GL bootstrap with the EGL-first ladder and the software-rendering
//! re-exec fallback), keyboard/mouse/wheel event handling and the
//! PTY event pump. Pure state lives in the sibling `app.rs`.

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::WindowId;

use super::app::App;
use super::env::{passwd_shell, shell_env};
use super::gl::{
    x11_window_id, CreatedGl, GlApi, GlProfile, GL_INIT_LADDER, GL_INIT_LADDER_ES_FIRST,
};
use crate::clip::Which;
use crate::input::{self, Key as InputKey};
use crate::pty::{Pty, PtyEvent};
use crate::render::Selection;
use crate::shell;

/// Lines per mouse-wheel notch.
const WHEEL_STEP: f32 = 3.0;

impl ApplicationHandler<PtyEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        // GL initialization ladder (Alacritty-style fallbacks): the
        // most capable configuration first, then progressively relaxed
        // ones, EGL first and GLX second (see `GlApi` for why EGL must
        // come first: GLX BadMatch during context creation never
        // triggers libglvnd's EGL fallback on its own).
        // `TUXIXI_GL_PREFER_ES=1` flips GLES 3.0 to the front for
        // machines whose drivers have no desktop core profile.
        let ladder: &[(bool, u8, GlApi, GlProfile, &str)] =
            if std::env::var_os("TUXIXI_GL_PREFER_ES").is_some() {
                &GL_INIT_LADDER_ES_FIRST
            } else {
                &GL_INIT_LADDER
            };
        let created = Self::try_gl_ladder(el, ladder);

        // Last resort: Mesa caches the chosen driver PER PROCESS, so
        // setting LIBGL_ALWAYS_SOFTWARE here cannot undo an already
        // probed hardware driver. Instead re-exec the binary in a
        // fresh process with the variable set: llvmpipe (the Mesa
        // software rasterizer) provides desktop GL 3.3 on any CPU,
        // and a slow terminal beats a terminal that refuses to start.
        if created.is_none() {
            eprintln!("no hardware configuration worked — retrying in a fresh process with software rendering (LIBGL_ALWAYS_SOFTWARE=1)");
            Self::reexec_with_software_gl();
        }
        let CreatedGl { window, backend } = match created {
            Some(c) => c,
            None => {
                eprintln!(
                    "fatal: no OpenGL configuration could be initialized.\n\
                     Tried desktop OpenGL 3.3, OpenGL ES 3.0 and forced software rendering.\n\
                     Hints: install/repair Mesa (libgl1-mesa-dri ships the software\n\
                     renderer llvmpipe; libegl1 + libegl-mesa0 provide EGL),\n\
                     and check that the compositor supports GLES (sway/wlroots does).\n\
                     If one rung works but not the others, report the log lines above."
                );
                self.exiting_cleanup();
                el.exit();
                return;
            }
        };
        self.window = Some(window);
        self.gl = Some(backend);

        let window = self.window.clone().expect("just set");
        self.x11_window_id = x11_window_id(&window);
        if self.glass.enabled {
            if let Some(id) = self.x11_window_id {
                let _ = crate::glass::request_blur(id);
            }
        }

        let size = window.inner_size();
        let (cols, rows) = self.grid_size_for(size.width.max(1), size.height.max(1));
        self.term.resize(cols, rows);

        // Welcome banner BEFORE the shell output arrives.
        self.feed_welcome();

        // Spawn the user's shell on a fresh PTY.
        let shell_path = shell::default_shell(passwd_shell().as_deref(), shell_env().as_deref());
        let argv = shell::shell_argv(&shell_path);
        let cwd = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        match Pty::spawn(&argv, &cwd, cols, rows, &Self::session_environment()) {
            Ok(pty) => {
                let proxy = self.proxy.clone();
                pty.start_reader(move |ev| {
                    let _ = proxy.send_event(ev);
                });
                pty.resize(cols, rows);
                self.pty = Some(pty);
            }
            Err(e) => eprintln!("failed to spawn shell: {e}"),
        }

        self.resize_all(size.width.max(1), size.height.max(1));
        window.request_redraw();
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: PtyEvent) {
        match event {
            PtyEvent::Output(bytes) => {
                if std::env::var_os("TUXIXI_DEBUG_INPUT").is_some() {
                    eprintln!("pty.out: {} bytes: {bytes:?}", bytes.len());
                }
                self.term.feed(&bytes);
                self.answer_term_events();
                self.term.maintain_memory();
                // Reflect OSC title changes in the window title.
                if let Some(win) = &self.window {
                    if !self.term.title.is_empty() {
                        let t = format!("TUXIXI Terminal — {}", self.term.title);
                        if win.title() != t {
                            win.set_title(&t);
                        }
                    }
                }
                if let Some(win) = &self.window {
                    win.request_redraw();
                }
            }
            PtyEvent::Exited => {
                self.exiting_cleanup();
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.exiting_cleanup();
                el.exit();
            }
            WindowEvent::Resized(size) => {
                self.resize_all(size.width.max(1), size.height.max(1));
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
            }
            WindowEvent::Focused(focused) => {
                // Modifier releases can land in another window (Alt+Tab,
                // menus) and leave the tracked snapshot stale; winit
                // re-delivers the current state on the next key event.
                if !focused {
                    self.mods = ModifiersState::empty();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                // Synthetic presses are replayed by the platform after
                // focus/mapping changes; they must never reach the child
                // or every window switch would inject phantom keys.
                if is_synthetic {
                    return;
                }
                let ctrl = self.mods.control_key();
                let shift = self.mods.shift_key();

                // Input tracing: set TUXIXI_DEBUG_INPUT=1 to log every
                // key event (physical/logical key, produced text, repeat
                // flag, tracked modifier state) and every byte sequence
                // written to the PTY. This is the ground truth needed to
                // diagnose shell-side "ghost input" reports: any byte
                // reaching the child that the user did not type shows up
                // verbatim here.
                let debug_input = std::env::var_os("TUXIXI_DEBUG_INPUT").is_some();
                if debug_input {
                    eprintln!(
                        "key: physical={:?} logical={:?} text={:?} repeat={} tracked={:?}",
                        event.physical_key, event.logical_key, event.text, event.repeat, self.mods
                    );
                }

                // Font size keys first (physical, layout-independent).
                if ctrl && !shift {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Equal | KeyCode::NumpadAdd) => {
                            self.change_font_size(1);
                            return;
                        }
                        PhysicalKey::Code(KeyCode::Minus | KeyCode::NumpadSubtract) => {
                            self.change_font_size(-1);
                            return;
                        }
                        _ => {}
                    }
                }

                let Some(key) = self.key_to_input_key(&event.logical_key) else {
                    return;
                };

                // X11 reports AltGr (right Alt) as the combined ctrl+alt
                // modifier state. A character produced through AltGr must
                // reach the child as plain text: keeping ctrl+alt here
                // would encode an ESC prefix and shells would interpret
                // the keystroke as a meta/vi binding instead of an
                // inserted character (stray characters, redrawn lines).
                let mods = input::strip_altgr(key, self.modifiers());
                let ctrl = mods.ctrl;
                let shift = mods.shift;

                // GUI shortcuts (Ctrl+Shift+C/V/T/G) run before the child.
                if ctrl && shift && self.handle_shortcut(&key) {
                    return;
                }

                // Shift+PageUp / Shift+PageDown scroll the buffer.
                if shift && !ctrl {
                    match key {
                        InputKey::PageUp => {
                            self.scroll_by(-(self.term.rows as isize));
                            return;
                        }
                        InputKey::PageDown => {
                            self.scroll_by(self.term.rows as isize);
                            return;
                        }
                        _ => {}
                    }
                }

                // Any other key input snaps back to the live screen.
                if self.scroll_offset != 0 {
                    self.scroll_offset = 0;
                }
                if let Some(bytes) = input::encode(key, mods, self.term.app_cursor()) {
                    if debug_input {
                        eprintln!(
                            "pty.write: mods={mods:?} -> {bytes:?} ({})",
                            String::from_utf8_lossy(&bytes)
                        );
                    }
                    if let Some(pty) = &self.pty {
                        pty.write(&bytes);
                    }
                }
                if let Some(win) = &self.window {
                    win.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x as i32, position.y as i32);
                if let Some(anchor) = self.select_anchor {
                    let cell = self.cell_at(self.mouse_pos.0, self.mouse_pos.1);
                    self.selection = Some(Selection {
                        start: anchor,
                        end: cell,
                    });
                    if let Some(win) = &self.window {
                        win.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                match button {
                    winit::event::MouseButton::Left => {
                        match state {
                            ElementState::Pressed => {
                                let cell = self.cell_at(self.mouse_pos.0, self.mouse_pos.1);
                                self.select_anchor = Some(cell);
                                self.selection = None;
                            }
                            ElementState::Released => {
                                if let Some(anchor) = self.select_anchor {
                                    let cell = self.cell_at(self.mouse_pos.0, self.mouse_pos.1);
                                    if anchor != cell {
                                        let sel = Selection {
                                            start: anchor,
                                            end: cell,
                                        };
                                        self.selection = Some(sel);
                                        // Drag-select copies immediately,
                                        // matching the mainstream terminal
                                        // behavior: PRIMARY is what other
                                        // X11 clients read for a
                                        // select-then-paste flow, and
                                        // CLIPBOARD is what desktop
                                        // applications paste with Ctrl+V,
                                        // so a plain selection is
                                        // pasteable in both worlds without
                                        // an explicit copy shortcut.
                                        let text = sel.text(&self.term);
                                        if !text.is_empty() {
                                            self.clipboards.store(Which::Primary, &text);
                                            self.clipboards.store(Which::Clipboard, &text);
                                        }
                                    }
                                    self.select_anchor = None;
                                    if let Some(win) = &self.window {
                                        win.request_redraw();
                                    }
                                }
                            }
                        }
                    }
                    winit::event::MouseButton::Middle => {
                        // Middle-click pastes the PRIMARY selection
                        // (the mouse selection), bracketed-paste
                        // encoded like every other paste path.
                        if state == ElementState::Pressed {
                            match self.clipboards.load(Which::Primary) {
                                Ok(text) => {
                                    let bytes =
                                        input::encode_paste(&text, self.term.bracketed_paste());
                                    if let Some(pty) = &self.pty {
                                        pty.write(&bytes);
                                    }
                                }
                                Err(e) => eprintln!("primary paste failed: {e}"),
                            }
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * WHEEL_STEP).round() as isize,
                    MouseScrollDelta::PixelDelta(p) => {
                        let lh = self.face.metrics.line_height.max(1) as f64;
                        (p.y / lh).round() as isize
                    }
                };
                if lines != 0 {
                    self.scroll_by(-lines);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        el.set_control_flow(ControlFlow::Wait);
    }
}
