//! The winit GUI of TUXIXI — a from-scratch rewrite of the GTK4 port.
//!
//! Why rewrite? The GTK build idled at ~240 MB: GLib main loop, the
//! GSK renderers, VTE's grid state, fontconfig/Pango caches and the
//! GObject type system all allocate on startup and never give it back.
//! Alacritty shows what a terminal can weigh, so the GUI is built the
//! same way Alacritty is:
//!
//! * windowing/event loop: `winit` (no toolkit, no main-loop runtime),
//! * rendering: `softbuffer` CPU framebuffer, repainted only when the
//!   PTY, the input or the scroll offset actually change,
//! * fonts: `ab_glyph`, rasterized on demand into a bounded cache,
//! * terminal emulation: our own grid + the `vte` parser (`crate::vt`),
//! * PTY: our own fork/exec (`crate::pty`),
//! * memory: every buffer is bounded and trimmed explicitly
//!   (`crate::mem`, `Terminal::memory_report`).
//!
//! Shortcuts:
//! * Ctrl+Shift+C / Ctrl+Shift+V — copy selection / paste
//! * Ctrl+Shift+T — cycle theme (persisted via the settings file)
//! * Ctrl+= / Ctrl+- — font size up / down (persisted)
//! * Shift+PageUp / Shift+PageDown, mouse wheel — scrollback
//!
//! Tabs were a GTK-only construct (one GObject stack per tab was a
//! large share of the idle memory); this build intentionally opens one
//! window per terminal, like Alacritty.

use std::sync::Arc;

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::font::{load_default_face, FontFace};
use crate::input::{self, Modifiers};
use crate::input::Key as InputKey;
use crate::pty::{kill_child, Pty, PtyEvent};
use crate::render::{paint, Palette, Selection};
use crate::settings::{BlurMode, Settings, SettingsManager};
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

struct App {
    settings: Settings,
    settings_mgr: SettingsManager,
    term: Terminal,
    pty: Option<Pty>,
    face: FontFace,
    palette: Palette,
    scroll_offset: usize,
    mods: ModifiersState,
    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    frame: Vec<u32>,
    selection: Option<Selection>,
    select_anchor: Option<(u16, u16)>,
    mouse_pos: (i32, i32),
    proxy: EventLoopProxy<PtyEvent>,
}

impl App {
    fn palette_from_settings(s: &Settings) -> Palette {
        let colors = theme::terminal_colors(&s.theme, false, BlurMode::Off.as_str());
        Palette::from_terminal_colors(&colors)
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
            // Manual memory: the frame buffer is one Vec, re-allocated
            // (freed) only on real resizes.
            let px_w = cols as usize * self.face.metrics.cell_width.max(1) as usize;
            let px_h = rows as usize * self.face.metrics.line_height.max(1) as usize;
            self.frame = vec![0u32; px_w * px_h];
        }
        if let Some(surface) = &mut self.surface {
            let _ = surface.resize(
                std::num::NonZeroU32::new(w.max(1)).unwrap(),
                std::num::NonZeroU32::new(h.max(1)).unwrap(),
            );
        }
        self.scroll_offset = 0;
        if let Some(win) = &self.window {
            win.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let Some(win) = &self.window else { return };
        let size = win.inner_size();
        let (w, h) = (size.width.max(1) as usize, size.height.max(1) as usize);
        if self.frame.len() != w * h {
            self.frame = vec![0u32; w * h];
        }
        paint(
            &mut self.frame,
            w,
            h,
            &self.term,
            self.scroll_offset,
            &self.palette,
            &mut self.face,
            self.selection.as_ref(),
        );
        if let Some(surface) = &mut self.surface {
            match surface.buffer_mut() {
                Ok(mut buffer) => {
                    // Copy only the overlapping prefix: the softbuffer
                    // buffer may be padded relative to our frame.
                    let n = buffer.len().min(self.frame.len());
                    buffer[..n].copy_from_slice(&self.frame[..n]);
                    let _ = buffer.present();
                }
                Err(e) => eprintln!("frame buffer error: {e:?}"),
            }
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
        let attrs = Window::default_attributes()
            .with_title("TUXIXI Terminal")
            .with_inner_size(PhysicalSize::new(INITIAL_SIZE.0, INITIAL_SIZE.1));
        let window = Arc::new(el.create_window(attrs).expect("create window"));

        let context = Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&context, window.clone()).expect("softbuffer surface");
        self.context = Some(context);
        self.surface = Some(surface);
        self.window = Some(window.clone());

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

                // GUI shortcuts (Ctrl+Shift+C/V/T) run before the child.
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
    /// Explicit teardown: close the PTY master, tell the child to go.
    fn exiting_cleanup(&mut self) {
        if let Some(pty) = &mut self.pty {
            pty.shutdown();
        }
        kill_child();
        // Release the big buffers explicitly instead of waiting for the
        // process to die with them.
        self.frame.clear();
        self.frame.shrink_to_fit();
        self.face.clear_cache();
        self.term.maintain_memory();
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

    let mut app = App {
        settings,
        settings_mgr,
        term,
        pty: None,
        face,
        palette,
        scroll_offset: 0,
        mods: ModifiersState::empty(),
        window: None,
        context: None,
        surface: None,
        frame: Vec::new(),
        selection: None,
        select_anchor: None,
        mouse_pos: (0, 0),
        proxy,
    };

    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop error: {e}");
    }
}
