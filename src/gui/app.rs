// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! The winit application state: one [`App`] per window owns the
//! terminal, the PTY, the font face, the GL backend and all input
//! handling (keyboard encoding, mouse selection, scrolling) plus the
//! GL context bootstrap with its software-rendering re-exec
//! fallback.

use glutin::config::ConfigTemplateBuilder;
use glutin::context::ContextAttributesBuilder;
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{GlSurface, WindowSurface};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use std::num::NonZeroU32;
use std::os::unix::process::CommandExt;
use std::sync::Arc;
use winit::dpi::PhysicalSize;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::Window;

use crate::font::{load_default_face, FontFace};
use crate::gpu::{build_scene, clear_color, Atlas, GlassParams, GpuRenderer, SceneInputs};

use super::env::bin_dir;
use super::gl::{panic_message, CreatedGl, GlApi, GlBackend, GlProfile};
use super::shortcuts::{shortcut_from_key, Shortcut};
use crate::input::Key as InputKey;
use crate::input::{self, Modifiers};
use crate::pty::{kill_child, Pty, PtyEvent};
use crate::render::{Palette, Selection};
use crate::settings::{Settings, SettingsManager};
use crate::shell;
use crate::theme;
use crate::vt::Terminal;

/// Initial window size in pixels.
const INITIAL_SIZE: (u32, u32) = (960, 600);
/// Font size bounds (mirrors the settings manager clamp).
const MIN_PX: i32 = 6;
const MAX_PX: i32 = 40;

pub(crate) struct App {
    pub(crate) settings: Settings,
    pub(crate) settings_mgr: SettingsManager,
    pub(crate) term: Terminal,
    pub(crate) pty: Option<Pty>,
    pub(crate) face: FontFace,
    pub(crate) palette: Palette,
    pub(crate) glass: GlassParams,
    pub(crate) scroll_offset: usize,
    pub(crate) mods: ModifiersState,
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) gl: Option<GlBackend>,
    /// X11 window id for the compositor blur request (glass mode).
    pub(crate) x11_window_id: Option<u32>,
    pub(crate) selection: Option<Selection>,
    pub(crate) select_anchor: Option<(u16, u16)>,
    pub(crate) mouse_pos: (i32, i32),
    pub(crate) proxy: EventLoopProxy<PtyEvent>,
    /// Long-lived clipboard provider. On X11, the clipboard is OWNED
    /// by the process: arboard serves the selection from a thread tied
    /// to this instance, so it must outlive every copy. Creating it per
    /// shortcut (the old approach) made the copied data evaporate the
    /// moment the handler returned — copy looked "done" but a later
    /// paste found nothing.
    pub(crate) clipboard: Option<arboard::Clipboard>,
}

impl App {
    /// One full GL initialization attempt: config selection + window
    /// creation + context + surface. Returns `Err` instead of panicking
    /// so the caller can retry with a relaxed configuration.
    ///
    /// The config must be chosen BEFORE the window so the correct
    /// visual/colormap is used (the GLX/EGL rule); `DisplayBuilder`
    /// handles that in one call.
    pub(crate) fn create_gl_backend(
        el: &ActiveEventLoop,
        transparent: bool,
        alpha_size: u8,
        api: GlApi,
        profile: GlProfile,
    ) -> Result<CreatedGl, String> {
        let attrs = Window::default_attributes()
            .with_title("TUXIXI Terminal")
            .with_transparent(transparent)
            .with_inner_size(PhysicalSize::new(INITIAL_SIZE.0, INITIAL_SIZE.1));

        // The API filter must match the requested context: EGL's
        // template default includes only the GLES2 bit, and a config
        // picked that way answers EGL_BAD_MATCH to a desktop context
        // (and vice versa). Desktop rungs filter for OPENGL, ES rungs
        // for GLES3.
        let (api_bits, context_api) = match profile {
            GlProfile::Desktop => (
                glutin::config::Api::OPENGL,
                glutin::context::ContextApi::OpenGl(Some(glutin::context::Version::new(3, 3))),
            ),
            GlProfile::Es3 => (
                glutin::config::Api::GLES3,
                glutin::context::ContextApi::Gles(Some(glutin::context::Version::new(3, 0))),
            ),
        };

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(alpha_size)
            .with_transparency(transparent)
            .with_api(api_bits);

        let preference = match api {
            GlApi::Egl => glutin_winit::ApiPreference::PreferEgl,
            GlApi::Glx => glutin_winit::ApiPreference::FallbackEgl,
        };
        let (window, gl_config) = DisplayBuilder::new()
            .with_window_attributes(Some(attrs))
            .with_preference(preference)
            .build(el, template, |mut configs| {
                configs
                    .next()
                    .expect("no compatible OpenGL framebuffer config")
            })
            .map_err(|e| format!("display/window creation: {e}"))?;
        let window = Arc::new(window.ok_or("DisplayBuilder created no window")?);

        let size = window.inner_size();
        let gl_display = gl_config.display();
        let raw_window = window
            .window_handle()
            .map_err(|e| format!("window handle: {e}"))?
            .as_raw();

        // OpenGL 3.3 core / GLES 3.0 is the renderer's requirement
        // (VAOs, instanced-style geometry); asking explicitly makes
        // the failure legible instead of silently handing back a
        // legacy context the shaders can't use.
        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(context_api)
            .build(Some(raw_window));
        let surface_attrs = glutin::surface::SurfaceAttributesBuilder::<WindowSurface>::new()
            .build(
                raw_window,
                NonZeroU32::new(size.width.max(1)).expect("width >= 1"),
                NonZeroU32::new(size.height.max(1)).expect("height >= 1"),
            );

        let not_current = unsafe { gl_display.create_context(&gl_config, &context_attrs) }
            .map_err(|e| format!("create context: {e:?}"))?;
        let surface = unsafe { gl_display.create_window_surface(&gl_config, &surface_attrs) }
            .map_err(|e| format!("create surface: {e:?}"))?;
        let context = not_current
            .make_current(&surface)
            .map_err(|e| format!("make current: {e:?}"))?;

        // Vsync off: the terminal only redraws when content changes,
        // so tearing on the rare updates costs less than the latency
        // of blocking swap.
        let _ = surface.set_swap_interval(&context, glutin::surface::SwapInterval::DontWait);

        let gl = Arc::new(unsafe {
            glow::Context::from_loader_function(|name| {
                let name = std::ffi::CString::new(name).unwrap();
                gl_display.get_proc_address(name.as_c_str()) as *const _
            })
        });
        let renderer = GpuRenderer::new(gl.clone()).map_err(|e| format!("renderer init: {e}"))?;

        Ok(CreatedGl {
            window,
            backend: GlBackend {
                renderer,
                atlas: Atlas::default(),
                surface,
                context,
            },
        })
    }

    /// Run the whole ladder top to bottom, reporting each failure and
    /// returning the first successful backend. `DisplayBuilder`'s
    /// config picker can only signal "no config" by panicking, so each
    /// rung is unwind-caught: a panic is treated like any other
    /// failure and the ladder moves on. The default panic hook is
    /// muted during the attempt so expected fallbacks don't print
    /// scary backtraces.
    pub(crate) fn try_gl_ladder(
        el: &ActiveEventLoop,
        ladder: &[(bool, u8, GlApi, GlProfile, &str)],
    ) -> Option<CreatedGl> {
        for &(transparent, alpha, api, profile, label) in ladder {
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Self::create_gl_backend(el, transparent, alpha, api, profile)
            }));
            std::panic::set_hook(prev_hook);
            match attempt {
                Ok(Ok(c)) => {
                    eprintln!("OpenGL: initialized with {label}");
                    return Some(c);
                }
                Ok(Err(e)) => {
                    eprintln!("OpenGL init failed ({label}): {e} — trying the next configuration")
                }
                Err(panic) => {
                    let msg = panic_message(&*panic);
                    eprintln!(
                        "OpenGL init failed ({label}): {msg} — trying the next configuration"
                    );
                }
            }
        }
        None
    }

    /// Re-exec this binary with Mesa forced to software rendering.
    ///
    /// `std::process::Command::exec` replaces the current process and
    /// never returns on success; the `TUXIXI_GL_SOFTWARE` marker keeps
    /// a second failure from looping forever. Returns only on failure
    /// (so the caller can continue to the fatal report).
    pub(crate) fn reexec_with_software_gl() {
        if std::env::var_os("TUXIXI_GL_SOFTWARE").is_some() {
            eprintln!("software-rendering retry already attempted — giving up");
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            eprintln!("cannot determine the binary path for the software-GL retry");
            return;
        };
        let err = std::process::Command::new(exe)
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env("TUXIXI_GL_SOFTWARE", "1")
            .exec();
        eprintln!("software-rendering re-exec failed: {err}");
    }

    pub(crate) fn palette_from_settings(s: &Settings) -> Palette {
        let colors = theme::terminal_colors(&s.theme, false, s.blur_mode.as_str());
        Palette::from_terminal_colors(&colors)
    }

    pub(crate) fn glass_from_settings(s: &Settings) -> GlassParams {
        GlassParams::new(s.glass, s.glass_opacity)
    }

    pub(crate) fn reload_face(&mut self) {
        let px = self.settings.font_size as f32;
        match load_default_face(&self.settings.font, px) {
            Ok(face) => self.face = face,
            Err(e) => eprintln!("font reload failed, keeping old face: {e}"),
        }
    }

    pub(crate) fn grid_size_for(&self, w: u32, h: u32) -> (u16, u16) {
        let cw = self.face.metrics.cell_width.max(1);
        let lh = self.face.metrics.line_height.max(1);
        let cols = ((w / cw) as u16).clamp(2, 500);
        let rows = ((h / lh) as u16).clamp(2, 200);
        (cols, rows)
    }

    pub(crate) fn resize_all(&mut self, w: u32, h: u32) {
        let (cols, rows) = self.grid_size_for(w, h);
        if cols != self.term.cols || rows != self.term.rows {
            self.term.resize(cols, rows);
            if let Some(pty) = &self.pty {
                pty.resize(cols, rows);
            }
        }
        if let Some(gl) = &self.gl {
            gl.resize(w.max(1), h.max(1));
        }
        self.scroll_offset = 0;
        if let Some(win) = &self.window {
            win.request_redraw();
        }
    }

    pub(crate) fn redraw(&mut self) {
        let Some(win) = &self.window else { return };
        let Some(gl) = &mut self.gl else { return };
        let size = win.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        // One scene build per repaint: the CPU walks the grid once and
        // produces quads; everything else happens on the GPU.
        let scene = build_scene(
            &self.term,
            self.scroll_offset,
            SceneInputs {
                palette: &self.palette,
                face: &mut self.face,
                atlas: &mut gl.atlas,
                selection: self.selection.as_ref(),
                glass: self.glass,
            },
        );
        let clear = clear_color(&self.palette, self.glass);
        gl.renderer
            .render(&scene, &mut gl.atlas, w as i32, h as i32, clear);
        gl.present();
    }

    /// Re-request (or remove) the compositor blur property after a
    /// glass toggle at runtime.
    pub(crate) fn apply_glass_to_compositor(&mut self) {
        if let Some(id) = self.x11_window_id {
            if self.glass.enabled {
                let outcome = crate::glass::request_blur(id);
                if outcome != crate::glass::BlurOutcome::Requested {
                    // Glass still renders (translucency), the
                    // compositor just does not blur. Not an error.
                }
            } else {
                crate::glass::remove_blur(id);
            }
        }
        if let Some(win) = &self.window {
            win.request_redraw();
        }
    }

    pub(crate) fn cell_at(&self, x: i32, y: i32) -> (u16, u16) {
        let cw = self.face.metrics.cell_width.max(1) as i32;
        let lh = self.face.metrics.line_height.max(1) as i32;
        let col = (x.max(0) / cw) as u16;
        let row = (y.max(0) / lh) as u16;
        let cell = (col.min(self.term.cols - 1), row.min(self.term.rows - 1));
        if std::env::var_os("TUXIXI_DEBUG_INPUT").is_some() {
            eprintln!("cell_at: ({x},{y}) -> {cell:?} (cw={cw} lh={lh})");
        }
        cell
    }

    pub(crate) fn scroll_by(&mut self, delta: isize) {
        if self.term.on_alt_screen() {
            return; // full-screen apps handle their own scrolling
        }
        let max = self.term.scrollback_len();
        let next = (self.scroll_offset as isize + delta).clamp(0, max as isize);
        if next as usize != self.scroll_offset {
            self.scroll_offset = next as usize;
            if let Some(win) = &self.window {
                win.request_redraw();
            }
        }
    }

    pub(crate) fn key_to_input_key(&self, key: &Key) -> Option<InputKey> {
        Some(match key {
            Key::Character(s) => InputKey::Char(s.chars().next()?),
            Key::Named(n) => match n {
                NamedKey::Enter => InputKey::Enter,
                NamedKey::Backspace => InputKey::Backspace,
                NamedKey::Tab => InputKey::Tab,
                NamedKey::Escape => InputKey::Escape,
                NamedKey::ArrowUp => InputKey::Up,
                NamedKey::ArrowDown => InputKey::Down,
                NamedKey::ArrowLeft => InputKey::Left,
                NamedKey::ArrowRight => InputKey::Right,
                NamedKey::Home => InputKey::Home,
                NamedKey::End => InputKey::End,
                NamedKey::PageUp => InputKey::PageUp,
                NamedKey::PageDown => InputKey::PageDown,
                NamedKey::Insert => InputKey::Insert,
                NamedKey::Delete => InputKey::Delete,
                NamedKey::F1 => InputKey::F(1),
                NamedKey::F2 => InputKey::F(2),
                NamedKey::F3 => InputKey::F(3),
                NamedKey::F4 => InputKey::F(4),
                NamedKey::F5 => InputKey::F(5),
                NamedKey::F6 => InputKey::F(6),
                NamedKey::F7 => InputKey::F(7),
                NamedKey::F8 => InputKey::F(8),
                NamedKey::F9 => InputKey::F(9),
                NamedKey::F10 => InputKey::F(10),
                NamedKey::F11 => InputKey::F(11),
                NamedKey::F12 => InputKey::F(12),
                NamedKey::Space => InputKey::Char(' '),
                _ => return None,
            },
            _ => return None,
        })
    }

    pub(crate) fn modifiers(&self) -> Modifiers {
        Modifiers {
            ctrl: self.mods.control_key(),
            alt: self.mods.alt_key(),
            shift: self.mods.shift_key(),
            logo: self.mods.super_key(),
        }
    }

    /// GUI shortcuts that run BEFORE bytes reach the child.
    /// Returns true when the key was consumed.
    pub(crate) fn handle_shortcut(&mut self, key: &InputKey) -> bool {
        if !(self.mods.control_key() && self.mods.shift_key()) {
            return false;
        }
        match shortcut_from_key(key) {
            Some(Shortcut::Copy) => {
                if let Some(sel) = &self.selection {
                    let text = sel.text(&self.term);
                    if std::env::var_os("TUXIXI_DEBUG_INPUT").is_some() {
                        eprintln!(
                            "copy: bounds={:?} text_len={} text={text:?}",
                            sel.bounds(),
                            text.len()
                        );
                    }
                    if text.is_empty() {
                        eprintln!("copy: selection is empty, nothing to copy");
                    } else {
                        match self.clipboard.as_mut() {
                            Some(clip) => {
                                if let Err(e) = clip.set_text(&text) {
                                    eprintln!("clipboard copy failed: {e}");
                                }
                            }
                            None => eprintln!("clipboard unavailable"),
                        }
                    }
                }
                true
            }
            Some(Shortcut::Paste) => {
                match self.clipboard.as_mut() {
                    Some(clip) => match clip.get_text() {
                        Ok(text) => {
                            let bytes = input::encode_paste(&text, self.term.bracketed_paste());
                            if let Some(pty) = &self.pty {
                                pty.write(&bytes);
                            }
                        }
                        Err(e) => eprintln!("clipboard paste failed: {e}"),
                    },
                    None => eprintln!("clipboard unavailable"),
                }
                true
            }
            Some(Shortcut::CycleTheme) => {
                if self.settings_mgr.cycle_theme() {
                    self.settings = self.settings_mgr.get().clone();
                    self.palette = Self::palette_from_settings(&self.settings);
                    if let Some(win) = &self.window {
                        win.request_redraw();
                    }
                }
                true
            }
            Some(Shortcut::ToggleGlass) => {
                if self.settings_mgr.toggle_glass() {
                    self.settings = self.settings_mgr.get().clone();
                    self.glass = Self::glass_from_settings(&self.settings);
                    self.apply_glass_to_compositor();
                }
                true
            }
            None => false,
        }
    }

    pub(crate) fn change_font_size(&mut self, delta: i32) {
        let current = self.settings.font_size as i32;
        let next = (current + delta).clamp(MIN_PX, MAX_PX);
        if next == current {
            return;
        }
        let target = next as u32;
        while self.settings_mgr.get().font_size != target {
            let step = if target > self.settings_mgr.get().font_size {
                1
            } else {
                -1
            };
            if !self.settings_mgr.change_font_size(step) {
                break;
            }
        }
        self.settings = self.settings_mgr.get().clone();
        self.reload_face();
        if let Some(win) = &self.window {
            let size = win.inner_size();
            self.resize_all(size.width, size.height);
        }
    }

    /// Build the session environment: inherited env with the TUXIXI
    /// bin dir merged into PATH, plus the TUXIXI_* markers.
    pub(crate) fn session_environment() -> Vec<(String, String)> {
        let bin_dir = bin_dir();
        let bin_dir_str = bin_dir.to_string_lossy().into_owned();
        let mut pairs: Vec<(String, String)> = shell::session_env(&bin_dir_str);
        if let Some(path) = std::env::var_os("PATH") {
            let merged = shell::merge_path(&path.to_string_lossy(), &bin_dir_str);
            pairs.retain(|(k, _)| k != "PATH");
            pairs.push(("PATH".to_string(), merged));
        }
        pairs
    }

    /// Feed the welcome banner (same `tuxixi --welcome` output as the
    /// GTK build, including the red `(rust)` tag).
    pub(crate) fn feed_welcome(&mut self) {
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(out) = std::process::Command::new(exe).arg("--welcome").output() {
                if out.status.success() && !out.stdout.is_empty() {
                    let text = String::from_utf8_lossy(&out.stdout).replace('\n', "\r\n");
                    self.term.feed(text.as_bytes());
                }
            }
        }
    }
}

impl App {
    /// Explicit teardown, mirroring the CPU renderer's manual memory
    /// hygiene: delete every GL object, release the GL context, close
    /// the PTY master, trim the terminal buffers.
    pub(crate) fn exiting_cleanup(&mut self) {
        if let Some(pty) = &mut self.pty {
            pty.shutdown();
        }
        kill_child();

        if let Some(gl) = self.gl.take() {
            let GlBackend {
                renderer: mut r,
                mut atlas,
                surface,
                context,
            } = gl;
            atlas.reset();
            r.destroy();
            drop(r);
            drop(atlas);
            // Release the current context before the surface dies.
            drop(context.make_not_current());
            drop(surface);
        }
        self.face.clear_cache();
        self.term.maintain_memory();
    }
}
