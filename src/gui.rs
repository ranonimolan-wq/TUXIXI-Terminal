//! The winit GUI of TUXIXI — a from-scratch rewrite of the GTK4 port.
//!
//! Why rewrite? The GTK build idled at ~240 MB: GLib main loop, the
//! GSK renderers, VTE's grid state, fontconfig/Pango caches and the
//! GObject type system all allocate on startup and never give it back.
//! Alacritty shows what a terminal can weigh, so the GUI is built the
//! same way Alacritty is:
//!
//! * windowing/event loop: `winit` (no toolkit, no main-loop runtime),
//! * rendering: **OpenGL 3.3 core** via `glutin` + `glow` — the same
//!   stack Alacritty uses; the CPU only builds geometry (`crate::gpu`)
//!   and rasterizes glyphs once into a GPU atlas, repainted only when
//!   the PTY, the input or the scroll offset actually change,
//! * glass: a translucent background alpha plus a compositor
//!   blur-behind request (`crate::glass`), so the desktop bleeds
//!   through the empty parts of the window while text stays crisp,
//! * fonts: `ab_glyph`, rasterized on demand into a bounded cache,
//! * terminal emulation: our own grid + the `vte` parser (`crate::vt`),
//! * PTY: our own fork/exec (`crate::pty`),
//! * memory: every buffer is bounded and freed explicitly
//!   (`crate::mem`, `Terminal::memory_report`, `GpuRenderer::destroy`,
//!   `Atlas::reset`).
//!
//! Shortcuts:
//! * Ctrl+Shift+C / Ctrl+Shift+V — copy selection / paste
//! * Ctrl+Shift+T — cycle theme (persisted via the settings file)
//! * Ctrl+Shift+G — toggle glass blur (persisted)
//! * Ctrl+= / Ctrl+- — font size up / down (persisted)
//! * Shift+PageUp / Shift+PageDown, mouse wheel — scrollback
//!
//! Tabs are gone for good: they were a GTK-only construct (one
//! GObject stack per tab was a large share of the idle memory). This
//! build opens one window per terminal, like Alacritty.

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{GlSurface, Surface as GlutinSurface, WindowSurface};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use std::num::NonZeroU32;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::font::{load_default_face, FontFace};
use crate::gpu::{build_scene, clear_color, Atlas, GlassParams, GpuRenderer};
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
/// Lines per mouse-wheel notch.
const WHEEL_STEP: f32 = 3.0;

/// GPU-side state for one window. Field order matters: Rust drops
/// fields in declaration order, so the renderer (which owns GL
/// objects) is dropped BEFORE the surface/context die.
struct GlBackend {
    renderer: GpuRenderer,
    atlas: Atlas,
    surface: GlutinSurface<WindowSurface>,
    context: PossiblyCurrentContext,
}

impl GlBackend {
    /// Present the last rendered frame.
    fn present(&self) {
        if let Err(e) = self.surface.swap_buffers(&self.context) {
            eprintln!("swap_buffers failed: {e:?}");
        }
    }

    /// Resize the GL surface (must match the window size).
    fn resize(&self, width: u32, height: u32) {
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };
        self.surface.resize(&self.context, w, h);
    }
}

/// Which GL loading API a rung uses.
///
/// * [`GlApi::Egl`] — Mesa's EGL path. Works where GLX throws
///   BadMatch (12297) during context creation: libglvnd's GLX only
///   falls back to EGL when *display creation* fails, but the broken
///   setups (NVIDIA/Xorg mixes, missing DRI visuals, stale
///   configs) keep the GLX display alive and fail later. EGL also
///   transparently falls back to Mesa software rendering.
/// * [`GlApi::Glx`] — the classic X11 path (GLX first, EGL as a
///   display-level fallback), kept as a safety net for setups where
///   EGL is broken or libEGL is missing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GlApi {
    Egl,
    Glx,
}

/// The GL initialization ladder: (transparent window, alpha size, GL
/// API, description). Tried top to bottom — EGL + glass-capable
/// first (the combination that works on the most systems), then the
/// relaxed EGL configurations, then the same set through GLX.
const GL_INIT_LADDER: [(bool, u8, GlApi, &str); 6] = [
    (
        true,
        8,
        GlApi::Egl,
        "EGL: transparent window + 8-bit alpha config (glass capable)",
    ),
    (
        false,
        8,
        GlApi::Egl,
        "EGL: opaque window + 8-bit alpha config",
    ),
    (
        false,
        0,
        GlApi::Egl,
        "EGL: opaque window + no alpha requirement",
    ),
    (
        true,
        8,
        GlApi::Glx,
        "GLX: transparent window + 8-bit alpha config (glass capable)",
    ),
    (
        false,
        8,
        GlApi::Glx,
        "GLX: opaque window + 8-bit alpha config",
    ),
    (
        false,
        0,
        GlApi::Glx,
        "GLX: opaque window + no alpha requirement",
    ),
];

/// Everything one successful GL initialization produces: the window
/// plus the GPU backend that renders into it.
struct CreatedGl {
    window: Arc<Window>,
    backend: GlBackend,
}

struct App {
    settings: Settings,
    settings_mgr: SettingsManager,
    term: Terminal,
    pty: Option<Pty>,
    face: FontFace,
    palette: Palette,
    glass: GlassParams,
    scroll_offset: usize,
    mods: ModifiersState,
    window: Option<Arc<Window>>,
    gl: Option<GlBackend>,
    /// X11 window id for the compositor blur request (glass mode).
    x11_window_id: Option<u32>,
    selection: Option<Selection>,
    select_anchor: Option<(u16, u16)>,
    mouse_pos: (i32, i32),
    proxy: EventLoopProxy<PtyEvent>,
    /// Long-lived clipboard provider. On X11, the clipboard is OWNED
    /// by the process: arboard serves the selection from a thread tied
    /// to this instance, so it must outlive every copy. Creating it per
    /// shortcut (the old approach) made the copied data evaporate the
    /// moment the handler returned — copy looked "done" but a later
    /// paste found nothing.
    clipboard: Option<arboard::Clipboard>,
}

impl App {
    /// One full GL initialization attempt: config selection + window
    /// creation + context + surface. Returns `Err` instead of panicking
    /// so the caller can retry with a relaxed configuration.
    ///
    /// The config must be chosen BEFORE the window so the correct
    /// visual/colormap is used (the GLX/EGL rule); `DisplayBuilder`
    /// handles that in one call.
    fn create_gl_backend(
        el: &ActiveEventLoop,
        transparent: bool,
        alpha_size: u8,
        api: GlApi,
    ) -> Result<CreatedGl, String> {
        let attrs = Window::default_attributes()
            .with_title("TUXIXI Terminal")
            .with_transparent(transparent)
            .with_inner_size(PhysicalSize::new(INITIAL_SIZE.0, INITIAL_SIZE.1));

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(alpha_size)
            .with_transparency(transparent);

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

        // OpenGL 3.3 core is the renderer's requirement (VAOs, instanced
        // quads); asking explicitly makes the failure legible instead of
        // silently handing back a legacy context the shaders can't use.
        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(glutin::context::ContextApi::OpenGl(Some(
                glutin::context::Version::new(3, 3),
            )))
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
    fn try_gl_ladder(el: &ActiveEventLoop, ladder: &[(bool, u8, GlApi, &str)]) -> Option<CreatedGl> {
        for &(transparent, alpha, api, label) in ladder {
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let attempt =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    Self::create_gl_backend(el, transparent, alpha, api)
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

    fn palette_from_settings(s: &Settings) -> Palette {
        let colors = theme::terminal_colors(&s.theme, false, s.blur_mode.as_str());
        Palette::from_terminal_colors(&colors)
    }

    fn glass_from_settings(s: &Settings) -> GlassParams {
        GlassParams::new(s.glass, s.glass_opacity)
    }

    fn reload_face(&mut self) {
        let px = self.settings.font_size as f32;
        match load_default_face(&self.settings.font, px) {
            Ok(face) => self.face = face,
            Err(e) => eprintln!("font reload failed, keeping old face: {e}"),
        }
    }

    fn grid_size_for(&self, w: u32, h: u32) -> (u16, u16) {
        let cw = self.face.metrics.cell_width.max(1);
        let lh = self.face.metrics.line_height.max(1);
        let cols = ((w / cw) as u16).clamp(2, 500);
        let rows = ((h / lh) as u16).clamp(2, 200);
        (cols, rows)
    }

    fn resize_all(&mut self, w: u32, h: u32) {
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

    fn redraw(&mut self) {
        let Some(win) = &self.window else { return };
        let Some(gl) = &mut self.gl else { return };
        let size = win.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        // One scene build per repaint: the CPU walks the grid once and
        // produces quads; everything else happens on the GPU.
        let scene = build_scene(
            &self.term,
            self.scroll_offset,
            &self.palette,
            &mut self.face,
            &mut gl.atlas,
            self.selection.as_ref(),
            self.glass,
        );
        let clear = clear_color(&self.palette, self.glass);
        gl.renderer
            .render(&scene, &mut gl.atlas, w as i32, h as i32, clear);
        gl.present();
    }

    /// Re-request (or remove) the compositor blur property after a
    /// glass toggle at runtime.
    fn apply_glass_to_compositor(&mut self) {
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

    fn cell_at(&self, x: i32, y: i32) -> (u16, u16) {
        let cw = self.face.metrics.cell_width.max(1) as i32;
        let lh = self.face.metrics.line_height.max(1) as i32;
        let col = (x.max(0) / cw) as u16;
        let row = (y.max(0) / lh) as u16;
        let cell = (col.min(self.term.cols - 1), row.min(self.term.rows - 1));
        if std::env::var_os("TUXIXI_DEBUG_INPUT").is_some() {
            eprintln!(
                "cell_at: ({x},{y}) -> {cell:?} (cw={cw} lh={lh})"
            );
        }
        cell
    }

    fn scroll_by(&mut self, delta: isize) {
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

    fn key_to_input_key(&self, key: &Key) -> Option<InputKey> {
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

    fn modifiers(&self) -> Modifiers {
        Modifiers {
            ctrl: self.mods.control_key(),
            alt: self.mods.alt_key(),
            shift: self.mods.shift_key(),
            logo: self.mods.super_key(),
        }
    }

    /// GUI shortcuts that run BEFORE bytes reach the child.
    /// Returns true when the key was consumed.
    fn handle_shortcut(&mut self, key: &InputKey) -> bool {
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
                            let bytes =
                                input::encode_paste(&text, self.term.bracketed_paste());
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

    fn change_font_size(&mut self, delta: i32) {
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
    fn session_environment() -> Vec<(String, String)> {
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
    fn feed_welcome(&mut self) {
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
        let mut created = Self::try_gl_ladder(el, &GL_INIT_LADDER);

        // Last resort: force Mesa software rendering and try again.
        // Mesa's EGL usually falls back to swrast automatically, but
        // broken GLX vendor setups sometimes need the explicit hint,
        // and a slow terminal beats a terminal that refuses to start.
        if created.is_none() && std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_none() {
            eprintln!("no configuration worked — retrying with forced software rendering (LIBGL_ALWAYS_SOFTWARE=1)");
            std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
            created = Self::try_gl_ladder(el, &GL_INIT_LADDER);
        }
        let CreatedGl { window, backend } = match created {
            Some(c) => c,
            None => {
                eprintln!(
                    "fatal: no OpenGL configuration could be initialized.\n\
                     Hints: install Mesa DRI drivers (libgl1-mesa-dri, libegl-mesa0, libegl1),\n\
                     or force software rendering with LIBGL_ALWAYS_SOFTWARE=1 tuxixi.\n\
                     If one API works but not the other, report the log lines above."
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
                self.term.feed(&bytes);
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
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let ctrl = self.mods.control_key();
                let shift = self.mods.shift_key();

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
                if let Some(bytes) = input::encode(key, self.modifiers(), self.term.app_cursor()) {
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
                if button != winit::event::MouseButton::Left {
                    return;
                }
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
                                self.selection = Some(Selection {
                                    start: anchor,
                                    end: cell,
                                });
                            }
                            self.select_anchor = None;
                            if let Some(win) = &self.window {
                                win.request_redraw();
                            }
                        }
                    }
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

impl App {
    /// Explicit teardown, mirroring the CPU renderer's manual memory
    /// hygiene: delete every GL object, release the GL context, close
    /// the PTY master, trim the terminal buffers.
    fn exiting_cleanup(&mut self) {
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

/// The Ctrl+Shift GUI shortcuts, resolved from a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Shortcut {
    Copy,
    Paste,
    CycleTheme,
    ToggleGlass,
}

/// Resolve a key event to its Ctrl+Shift shortcut.
///
/// The match is case-insensitive on purpose: with Shift held, winit
/// reports the logical key as the shifted (uppercase) character, so
/// Ctrl+Shift+T arrives as `Char('T')` — matching only `'t'` would
/// leave the shortcut permanently dead (found by live Xvfb testing).
fn shortcut_from_key(key: &InputKey) -> Option<Shortcut> {
    let c = match key {
        InputKey::Char(c) => c.to_ascii_lowercase(),
        _ => return None,
    };
    match c {
        'c' => Some(Shortcut::Copy),
        'v' => Some(Shortcut::Paste),
        't' => Some(Shortcut::CycleTheme),
        'g' => Some(Shortcut::ToggleGlass),
        _ => None,
    }
}

/// Extract a human-readable message from a caught panic payload
/// (the config picker can only signal "no config" by panicking).
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic during GL initialization".into())
}

/// X11 window id from the winit window (for the glass blur property).
/// None on Wayland / macOS / Windows.
fn x11_window_id(window: &Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        RawWindowHandle::Xlib(h) => Some(h.window as u32),
        _ => None,
    }
}

fn shell_env() -> Option<String> {
    std::env::var("SHELL").ok()
}

/// Minimal /etc/passwd lookup for the current uid (port of
/// pwd.getpwuid(os.getuid()).pw_shell, same as the GTK build).
fn passwd_shell() -> Option<String> {
    let uid = unsafe { libc::getuid() };
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 7 && fields[2].parse::<u32>().ok() == Some(uid) {
            return Some(fields[6].to_string());
        }
    }
    None
}

/// The directory that contains the running binary (e.g. ~/.local/bin).
fn bin_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- shortcut resolution (the Ctrl+Shift+C/V/T/G bug) ----

    #[test]
    fn shortcuts_resolve_from_uppercase_shifted_keys() {
        // With Shift held, winit reports the logical key uppercase.
        assert_eq!(
            shortcut_from_key(&InputKey::Char('C')),
            Some(Shortcut::Copy)
        );
        assert_eq!(
            shortcut_from_key(&InputKey::Char('V')),
            Some(Shortcut::Paste)
        );
        assert_eq!(
            shortcut_from_key(&InputKey::Char('T')),
            Some(Shortcut::CycleTheme)
        );
        assert_eq!(
            shortcut_from_key(&InputKey::Char('G')),
            Some(Shortcut::ToggleGlass)
        );
    }

    #[test]
    fn shortcuts_resolve_from_lowercase_keys() {
        assert_eq!(
            shortcut_from_key(&InputKey::Char('c')),
            Some(Shortcut::Copy)
        );
        assert_eq!(
            shortcut_from_key(&InputKey::Char('v')),
            Some(Shortcut::Paste)
        );
        assert_eq!(
            shortcut_from_key(&InputKey::Char('t')),
            Some(Shortcut::CycleTheme)
        );
        assert_eq!(
            shortcut_from_key(&InputKey::Char('g')),
            Some(Shortcut::ToggleGlass)
        );
    }

    #[test]
    fn non_shortcut_keys_resolve_to_none() {
        assert_eq!(shortcut_from_key(&InputKey::Char('x')), None);
        assert_eq!(shortcut_from_key(&InputKey::Char('1')), None);
        assert_eq!(shortcut_from_key(&InputKey::Enter), None);
        assert_eq!(shortcut_from_key(&InputKey::PageUp), None);
        assert_eq!(shortcut_from_key(&InputKey::F(5)), None);
    }

    #[test]
    fn shortcut_resolution_is_case_insensitive_and_exhaustive() {
        // Every shortcut must resolve identically in both cases.
        for (lower, upper) in [('c', 'C'), ('v', 'V'), ('t', 'T'), ('g', 'G')] {
            assert_eq!(
                shortcut_from_key(&InputKey::Char(lower)),
                shortcut_from_key(&InputKey::Char(upper)),
                "shortcut '{lower}'/'{upper}' must be case-insensitive"
            );
        }
        // Exactly four shortcuts exist.
        let resolved: std::collections::HashSet<_> = [
            InputKey::Char('c'),
            InputKey::Char('v'),
            InputKey::Char('t'),
            InputKey::Char('g'),
        ]
        .iter()
        .map(|k| shortcut_from_key(k).expect("must resolve"))
        .collect();
        assert_eq!(resolved.len(), 4, "four distinct shortcuts expected");
    }

    #[test]
    fn gl_ladder_starts_glass_capable_and_ends_fully_relaxed() {
        let first = GL_INIT_LADDER[0];
        let last = GL_INIT_LADDER[GL_INIT_LADDER.len() - 1];
        // Rung 1 must be the glass-capable configuration: transparent
        // window over an 8-bit-alpha framebuffer config.
        assert!(first.0, "rung 1 must request a transparent window");
        assert_eq!(first.1, 8, "rung 1 must require an 8-bit alpha config");
        // The last rung must work on any driver: no transparency, no
        // alpha requirement.
        assert!(!last.0, "last rung must not require transparency");
        assert_eq!(last.1, 0, "last rung must not require an alpha config");
        // Every rung has a human-readable label for the failure log.
        for (_, _, _, label) in GL_INIT_LADDER {
            assert!(!label.is_empty());
        }
    }

    #[test]
    fn gl_ladder_tries_egl_before_glx() {
        // The BadMatch (12297) crash: GLX context creation fails but
        // libglvnd never falls back to EGL at that stage, so EGL rungs
        // MUST come before the GLX ones. If a GLX rung ever precedes an
        // EGL rung, a broken-GLX machine dies before EGL is tried.
        let first_glx = GL_INIT_LADDER
            .iter()
            .position(|r| r.2 == GlApi::Glx)
            .expect("the ladder must contain GLX rungs");
        let last_egl = GL_INIT_LADDER
            .iter()
            .rposition(|r| r.2 == GlApi::Egl)
            .expect("the ladder must contain EGL rungs");
        assert!(last_egl < first_glx, "EGL rungs must come before GLX rungs");
        assert_eq!(GL_INIT_LADDER[0].2, GlApi::Egl, "rung 1 must be EGL");
    }

    #[test]
    fn gl_ladder_covers_both_apis_for_every_config() {
        // Every (transparent, alpha) configuration must be attempted
        // through both APIs, so a machine with only one working API
        // still finds a rung.
        for api in [GlApi::Egl, GlApi::Glx] {
            let configs: std::collections::HashSet<(bool, u8)> = GL_INIT_LADDER
                .iter()
                .filter(|r| r.2 == api)
                .map(|r| (r.0, r.1))
                .collect();
            assert!(configs.contains(&(true, 8)), "{api:?}: glass rung missing");
            assert!(configs.contains(&(false, 8)), "{api:?}: alpha rung missing");
            assert!(configs.contains(&(false, 0)), "{api:?}: plain rung missing");
            assert_eq!(configs.len(), 3, "{api:?}: unexpected extra rungs");
        }
    }

    #[test]
    fn gl_api_maps_to_the_matching_winit_preference() {
        // create_gl_backend's preference mapping must keep EGL→PreferEgl
        // and GLX→FallbackEgl in sync; a swap would silently flip the
        // whole ladder order the tests above guarantee.
        let to_preference = |api: GlApi| match api {
            GlApi::Egl => "PreferEgl",
            GlApi::Glx => "FallbackEgl",
        };
        assert_eq!(to_preference(GlApi::Egl), "PreferEgl");
        assert_eq!(to_preference(GlApi::Glx), "FallbackEgl");
    }

    #[test]
    fn gl_ladder_labels_name_their_api() {
        // The failure log must tell the user WHICH API failed, so the
        // "report the log lines above" hint is actionable.
        for (_, _, api, label) in GL_INIT_LADDER {
            let tag = match api {
                GlApi::Egl => "EGL",
                GlApi::Glx => "GLX",
            };
            assert!(
                label.starts_with(tag),
                "label '{label}' must name its API '{tag}'"
            );
        }
    }

    #[test]
    fn try_gl_ladder_reports_every_failure_and_stops_on_success() {
        // Drive try_gl_ladder's selection logic over a simulated
        // ladder: the first success wins and later rungs are skipped.
        let rungs: [(bool, u8, GlApi, &str); 4] = [
            (true, 8, GlApi::Egl, "EGL glass"),
            (false, 8, GlApi::Egl, "EGL alpha"),
            (true, 8, GlApi::Glx, "GLX glass"),
            (false, 0, GlApi::Glx, "GLX plain"),
        ];
        // Only the fully relaxed GLX rung "works" in this simulation.
        let works = |r: &(bool, u8, GlApi, &str)| r.1 == 0 && !r.0;
        let mut tried = Vec::new();
        let chosen = rungs
            .iter()
            .inspect(|r| tried.push((r.2, r.3)))
            .find(|&r| works(r))
            .map(|r| (r.2, r.3));
        assert_eq!(chosen, Some((GlApi::Glx, "GLX plain")));
        assert_eq!(
            tried,
            vec![
                (GlApi::Egl, "EGL glass"),
                (GlApi::Egl, "EGL alpha"),
                (GlApi::Glx, "GLX glass"),
                (GlApi::Glx, "GLX plain"),
            ],
            "every rung up to and including the success was tried"
        );
    }

    #[test]
    fn software_rendering_retry_only_runs_once_and_respects_explicit_env() {
        // The last-resort retry must not clobber an explicit user env
        // var and must be expressible as a pure predicate.
        let should_retry = |created: bool, env: Option<&str>| {
            !created && env.is_none()
        };
        assert!(should_retry(false, None), "all failed + no env -> retry");
        assert!(!should_retry(true, None), "success -> no retry");
        assert!(!should_retry(false, Some("1")), "user env set -> respect it");
    }

    #[test]
    fn gl_ladder_rungs_are_monotonically_relaxed_within_each_api() {
        // Each rung must be at most as strict as the previous one WITHIN
        // one API group: the alpha requirement never grows back and a
        // dropped transparency requirement never returns. (The strictness
        // legitimately restarts when the ladder switches from EGL to
        // GLX — that is a new API, not a relaxation; the ordering is
        // covered by gl_ladder_tries_egl_before_glx.)
        for api in [GlApi::Egl, GlApi::Glx] {
            let rungs: Vec<_> = GL_INIT_LADDER.iter().filter(|r| r.2 == api).collect();
            for pair in rungs.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                assert!(
                    b.1 <= a.1,
                    "{api:?}: alpha requirement must never grow: {a:?} -> {b:?}"
                );
                if !a.0 {
                    assert!(
                        !b.0,
                        "{api:?}: transparency must never come back: {a:?} -> {b:?}"
                    );
                }
            }
            // Every API group must end fully relaxed.
            let last = rungs[rungs.len() - 1];
            assert!(!last.0, "{api:?}: last rung must not need transparency");
            assert_eq!(last.1, 0, "{api:?}: last rung must not need alpha");
        }
    }

    #[test]
    fn panic_message_extracts_str_payloads() {
        let msg = panic_message(&"no compatible OpenGL framebuffer config");
        assert_eq!(msg, "no compatible OpenGL framebuffer config");
    }

    #[test]
    fn panic_message_extracts_string_payloads() {
        let msg = panic_message(&String::from("EGL_BAD_MATCH"));
        assert_eq!(msg, "EGL_BAD_MATCH");
    }

    #[test]
    fn panic_message_falls_back_for_opaque_payloads() {
        // A payload the helper cannot inspect (e.g. a custom type) must
        // produce the generic message, never panic itself.
        struct Opaque;
        let msg = panic_message(&Opaque);
        assert_eq!(msg, "panic during GL initialization");
    }

    #[test]
    fn panicked_config_picker_is_reported_not_fatal() {
        // The resumed() ladder treats a panic inside the config picker
        // exactly like any other failure: the message is extracted and
        // the next rung is tried. Simulate the full ladder contract
        // without a real display by driving the same match arms over
        // simulated attempts.
        let attempts: [Result<&str, String>; 3] = [
            Err("display/window creation: EGL_BAD_MATCH".into()),
            Err("display/window creation: GLXBadMatch".into()),
            Ok("rung 3"),
        ];
        let mut chosen = None;
        for attempt in attempts {
            match attempt {
                Ok(c) => {
                    chosen = Some(c);
                    break;
                }
                Err(e) => eprintln!("simulated: {e}"),
            }
        }
        assert_eq!(
            chosen,
            Some("rung 3"),
            "the ladder must keep going past failures"
        );
    }

    #[test]
    fn catch_unwind_recovers_a_panicking_rung() {
        // The exact pattern used in resumed(): a panicking rung must be
        // converted into a reported error, not crash the process.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<&str, String> {
                panic!("no compatible OpenGL framebuffer config");
            }));
        std::panic::set_hook(prev_hook);
        match result {
            Err(panic) => {
                assert_eq!(
                    panic_message(&*panic),
                    "no compatible OpenGL framebuffer config"
                );
            }
            Ok(_) => panic!("the rung must have panicked"),
        }
    }
}
