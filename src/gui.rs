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
use std::num::NonZeroU32;
use std::sync::Arc;
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::font::{load_default_face, FontFace};
use crate::gpu::{build_scene, clear_color, Atlas, GpuRenderer, GlassParams};
use crate::input::{self, Modifiers};
use crate::input::Key as InputKey;
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
}

impl App {
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
        gl.renderer.render(
            &scene,
            &mut gl.atlas,
            w as i32,
            h as i32,
            clear,
        );
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
        (col.min(self.term.cols - 1), row.min(self.term.rows - 1))
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
        match key {
            InputKey::Char('c') => {
                if let Some(sel) = &self.selection {
                    let text = sel.text(&self.term);
                    if let Ok(mut clip) = arboard::Clipboard::new() {
                        let _ = clip.set_text(text);
                    }
                }
                true
            }
            InputKey::Char('v') => {
                if let Ok(mut clip) = arboard::Clipboard::new() {
                    if let Ok(text) = clip.get_text() {
                        let bytes = input::encode_paste(&text, self.term.bracketed_paste());
                        if let Some(pty) = &self.pty {
                            pty.write(&bytes);
                        }
                    }
                }
                true
            }
            InputKey::Char('t') => {
                if self.settings_mgr.cycle_theme() {
                    self.settings = self.settings_mgr.get().clone();
                    self.palette = Self::palette_from_settings(&self.settings);
                    if let Some(win) = &self.window {
                        win.request_redraw();
                    }
                }
                true
            }
            InputKey::Char('g') => {
                if self.settings_mgr.toggle_glass() {
                    self.settings = self.settings_mgr.get().clone();
                    self.glass = Self::glass_from_settings(&self.settings);
                    self.apply_glass_to_compositor();
                }
                true
            }
            _ => false,
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

        // The window is ALWAYS created with an alpha-capable visual so
        // the glass mode can be toggled at runtime; when glass is off
        // the renderer clears fully opaque, which looks identical to a
        // plain window.
        let attrs = Window::default_attributes()
            .with_title("TUXIXI Terminal")
            .with_transparent(true)
            .with_inner_size(PhysicalSize::new(INITIAL_SIZE.0, INITIAL_SIZE.1));

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_transparency(true);

        // One call: GL display + config selection + window creation
        // (the config must be chosen BEFORE the window so the correct
        // visual/colormap is used — the GLX rule).
        let (window, gl_config) = DisplayBuilder::new()
            .with_window_attributes(Some(attrs))
            .build(el, template, |mut configs| {
                configs
                    .next()
                    .expect("no compatible OpenGL config found")
            })
            .expect("create GL window");
        let window = Arc::new(window.expect("DisplayBuilder must create a window"));
        self.window = Some(window.clone());

        let size = window.inner_size();
        let gl_display = gl_config.display();
        let raw_window = window.window_handle().expect("window handle").as_raw();
        let context_attrs = ContextAttributesBuilder::new().build(Some(raw_window));
        let surface_attrs = glutin::surface::SurfaceAttributesBuilder::<WindowSurface>::new()
            .build(
                raw_window,
                NonZeroU32::new(size.width.max(1)).unwrap(),
                NonZeroU32::new(size.height.max(1)).unwrap(),
            );

        let not_current = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .expect("create OpenGL context")
        };
        let surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attrs)
                .expect("create GL window surface")
        };

        let context = not_current.make_current(&surface).expect("make GL context current");

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
        let renderer = GpuRenderer::new(gl.clone()).expect("initialize GL renderer");

        self.gl = Some(GlBackend {
            renderer,
            atlas: Atlas::default(),
            surface,
            context,
        });

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
                if let Some(bytes) = input::encode(key, self.modifiers(), self.term.app_cursor())
                {
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
    };

    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop error: {e}");
    }
}
