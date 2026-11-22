//! GTK4 GUI port of `main.py`, `ui.py` and `terminal.py`.
//!
//! Requires the `gui` feature (gtk4 + vte4 crates and the matching
//! system libraries: GTK4 and VTE 3.91):
//!
//! ```sh
//! cargo build --release --features gui
//! ```
//!
//! This module mirrors the Python application:
//! * one `Gtk.Application` (`org.tuxixi.Terminal`) with global CSS,
//! * a window with a slim tab bar (`+` button, blur badge),
//! * per-tab independent VTE sessions spawning the user's shell,
//! * all the Ctrl+Shift / Alt keyboard shortcuts,
//! * glass / static blur / smoky blur modes driven by settings.
//!
//! The headless test suite cannot exercise widgets, so this module is
//! written conservatively using only core gtk4-rs/vte4 APIs.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Button, CssProvider, EventControllerKey, EventControllerMotion,
    GestureClick, Label, Orientation, Overlay, Picture, PolicyType, ScrolledWindow, Stack,
};
use vte4::prelude::*;
use vte4::Terminal as VteTerminal;

use crate::settings::{SettingsManager, BlurMode, Settings};
use crate::shell;
#[allow(unused_imports)]
use crate::theme;
use crate::title;
use crate::wallpaper;

pub const APP_ID: &str = "org.tuxixi.Terminal";

struct Tab {
    vte: VteTerminal,
    widget: gtk4::Box,
    item: TabItem,
}

/// Shared window state, owned through `Rc<RefCell<...>>` because the
/// widgets and their closures all reference it.
struct TuxixiWindow {
    window: ApplicationWindow,
    static_bg_container: gtk4::Box,
    tabs_box: gtk4::Box,
    tabs_scroller: ScrolledWindow,
    stack: Stack,
    blur_badge: Label,
    settings: Rc<RefCell<SettingsManager>>,
    tabs: Vec<Tab>,
    uid_seq: usize,
}

type WindowRef = Rc<RefCell<TuxixiWindow>>;

impl TuxixiWindow {
    fn build(app: &Application, settings: Rc<RefCell<SettingsManager>>) -> WindowRef {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("TUXIXI")
            .default_width(700)
            .default_height(450)
            .build();

        let overlay = Overlay::new();
        window.set_child(Some(&overlay));

        // Static blur background layer (hidden by default).
        let static_bg_container = gtk4::Box::new(Orientation::Vertical, 0);
        static_bg_container.set_hexpand(true);
        static_bg_container.set_vexpand(true);
        static_bg_container.set_can_target(false);
        static_bg_container.set_visible(false);
        overlay.set_child(Some(&static_bg_container));
        setup_static_blur_layer(&static_bg_container);

        let root_box = gtk4::Box::new(Orientation::Vertical, 0);
        root_box.set_hexpand(true);
        root_box.set_vexpand(true);
        overlay.add_overlay(&root_box);

        let tabbar = gtk4::Box::new(Orientation::Horizontal, 4);
        tabbar.add_css_class("tuxixi-tabbar");

        let tabs_scroller = ScrolledWindow::new();
        tabs_scroller.set_policy(PolicyType::External, PolicyType::Never);
        tabs_scroller.set_hexpand(true);

        let tabs_box = gtk4::Box::new(Orientation::Horizontal, 4);
        tabs_scroller.set_child(Some(&tabs_box));
        tabbar.append(&tabs_scroller);

        let blur_badge = Label::new(None);
        blur_badge.add_css_class("tuxixi-blur-badge");
        blur_badge.set_visible(false);
        tabbar.append(&blur_badge);

        let add_tab_btn = Button::with_label("+");
        add_tab_btn.add_css_class("tuxixi-tab-add");
        add_tab_btn.set_can_focus(false);
        add_tab_btn.set_tooltip_text(Some("New Tab (Ctrl+Shift+Enter)"));

        root_box.append(&tabbar);

        let stack = Stack::new();
        stack.set_transition_type(gtk4::StackTransitionType::None);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        root_box.append(&stack);

        let this: WindowRef = Rc::new(RefCell::new(TuxixiWindow {
            window,
            static_bg_container,
            tabs_box,
            tabs_scroller,
            stack,
            blur_badge,
            settings,
            tabs: Vec::new(),
            uid_seq: 0,
        }));

        // "+" button -> new independent tab (every tab starts at ~).
        {
            let this = this.clone();
            add_tab_btn.connect_clicked(move |_| {
                Self::add_tab(&this);
            });
        }
        tabbar.append(&add_tab_btn);

        // First tab + initial blur badge state.
        Self::add_tab(&this);
        {
            let s0 = this.borrow().settings.borrow().get().clone();
            this.borrow().apply_blur_state(&s0);
            this.borrow().apply_fonts(&s0);
        }

        // Live visual updates: whenever any setting actually changes
        // (theme / glass / blur / font), refresh the blur badge and the
        // terminal fonts. The global CSS is refreshed by its own
        // listener registered in `apply_global_css`. The listener gets
        // the new Settings snapshot, so it never re-borrows `settings`
        // (which is still mutably borrowed inside `cycle_theme` etc.).
        {
            let this = this.clone();
            let settings_rc = this.borrow().settings.clone();
            settings_rc.borrow_mut().on_change(std::rc::Rc::new(move |s| {
                let b = this.borrow();
                b.apply_blur_state(s);
                b.apply_fonts(s);
            }));
        }

        this
    }

    fn add_tab(this: &WindowRef) {
        let (uid, tab_number) = {
            let mut b = this.borrow_mut();
            b.uid_seq += 1;
            (b.uid_seq, b.tabs.len() + 1)
        };
        let page_name = format!("tab-{uid}");

        let vte = VteTerminal::new();
        vte.set_hexpand(true);
        vte.set_vexpand(true);
        vte.set_scrollback_lines(10000);
        vte.set_cursor_blink_mode(vte4::CursorBlinkMode::System);
        vte.set_cursor_shape(vte4::CursorShape::Block);

        let widget = gtk4::Box::new(Orientation::Vertical, 0);
        widget.append(&vte);

        let item = TabItem::new(&format!("{tab_number}: ~"));

        // Clicking the tab selects it.
        {
            let this = this.clone();
            let vte_for_select = vte.clone();
            item.set_on_select(move || {
                Self::select_tab(&this, &vte_for_select);
            });
        }
        // Closing (button or middle click).
        {
            let this = this.clone();
            let vte_for_close = vte.clone();
            item.set_on_close(move || {
                Self::close_tab(&this, &vte_for_close);
            });
        }

        this.borrow_mut().tabs_box.append(&item.widget);

        this.borrow_mut().tabs.push(Tab {
            vte: vte.clone(),
            widget: widget.clone(),
            item,
        });
        this.borrow_mut().stack.add_named(&widget, Some(&page_name));

        // Terminal behaviors: shortcuts, shell session, title tracking.
        setup_key_controller(&vte, this.clone());
        setup_session(&vte);
        {
            let this = this.clone();
            vte.connect_window_title_changed(move |_| {
                Self::refresh_tab_titles(&this);
            });
        }
        // Exiting the shell (e.g. `exit`, Ctrl+D) closes the tab.
        {
            let this = this.clone();
            let vte_for_exit = vte.clone();
            vte.connect_child_exited(move |_, _status| {
                Self::close_tab(&this, &vte_for_exit);
            });
        }

        Self::refresh_tab_titles(this);
        Self::select_tab(this, &vte);
    }

    fn find_index(borrowed: &TuxixiWindow, vte: &VteTerminal) -> Option<usize> {
        borrowed.tabs.iter().position(|t| t.vte == *vte)
    }

    fn select_tab(this: &WindowRef, vte: &VteTerminal) {
        let index = {
            let b = this.borrow();
            Self::find_index(&b, vte)
        };
        let Some(index) = index else { return };
        let b = this.borrow();
        for (i, tab) in b.tabs.iter().enumerate() {
            tab.item.set_active(i == index);
        }
        let tab = &b.tabs[index];
        b.stack.set_visible_child(&tab.widget);
        tab.vte.grab_focus();
        let dyn_title = dynamic_title(&tab.vte);
        b.window
            .set_title(Some(&title::window_title(Some(&dyn_title))));
        let hadj = b.tabs_scroller.hadjustment();
        let max = (hadj.upper() - hadj.page_size()).max(hadj.lower());
        hadj.set_value(max);
    }

    fn switch_tab_relative(this: &WindowRef, delta: i32) {
        let len = this.borrow().tabs.len();
        if len <= 1 {
            return;
        }
        let active = this.borrow().stack.visible_child();
        let current = this
            .borrow()
            .tabs
            .iter()
            .position(|t| Some(t.widget.clone().upcast::<gtk4::Widget>()) == active)
            .unwrap_or(0);
        let next = ((current as i32 + delta).rem_euclid(len as i32)) as usize;
        let vte = this.borrow().tabs[next].vte.clone();
        Self::select_tab(this, &vte);
    }

    fn select_tab_by_index(this: &WindowRef, index: i32) {
        let vte = {
            let b = this.borrow();
            if index < 0 {
                return;
            }
            b.tabs.get(index as usize).map(|t| t.vte.clone())
        };
        if let Some(vte) = vte {
            Self::select_tab(this, &vte);
        }
    }

    fn close_tab(this: &WindowRef, vte: &VteTerminal) {
        let index = {
            let b = this.borrow();
            Self::find_index(&b, vte)
        };
        let Some(index) = index else { return };

        let was_active = {
            let b = this.borrow();
            b.stack
                .visible_child()
                .map(|w| w == b.tabs[index].widget.clone().upcast::<gtk4::Widget>())
                .unwrap_or(false)
        };

        {
            let mut b = this.borrow_mut();
            let tab = b.tabs.remove(index);
            b.tabs_box.remove(&tab.item.widget);
            b.stack.remove(&tab.widget);
        }

        if this.borrow().tabs.is_empty() {
            this.borrow().window.close();
            return;
        }

        Self::refresh_tab_titles(this);

        if was_active {
            let next_index = index.min(this.borrow().tabs.len() - 1);
            Self::select_tab_by_index(this, next_index as i32);
        }
    }

    fn refresh_tab_titles(this: &WindowRef) {
        let b = this.borrow();
        let active = b.stack.visible_child();
        for (i, tab) in b.tabs.iter().enumerate() {
            let dyn_title = dynamic_title(&tab.vte);
            let display = title::tab_display_title(i + 1, Some(&dyn_title));
            tab.item.set_title(&display);
            if active == Some(tab.widget.clone().upcast::<gtk4::Widget>()) {
                b.window
                    .set_title(Some(&title::window_title(Some(&dyn_title))));
            }
        }
    }

    fn apply_blur_state(&self, s: &Settings) {
        self.static_bg_container
            .set_visible(s.blur_mode == BlurMode::Static);
        match s.blur_mode {
            BlurMode::Static => {
                self.blur_badge.set_text("◆ Blur: Static");
                self.blur_badge.set_visible(true);
            }
            BlurMode::Smoky => {
                self.blur_badge.set_text("◆ Blur: Smoky");
                self.blur_badge.set_visible(true);
            }
            BlurMode::Off => self.blur_badge.set_visible(false),
        }
    }

    /// Apply the configured font (family + size) to every open terminal.
    /// Called on startup and on every settings change, so Ctrl+Shift+=
    /// and Ctrl+Shift+- resize the terminals immediately.
    fn apply_fonts(&self, s: &Settings) {
        let desc =
            gtk4::pango::FontDescription::from_string(&format!("{} {}", s.font, s.font_size));
        for tab in &self.tabs {
            tab.vte.set_font(Some(&desc));
        }
    }
}

/// Port of `TuxixiTerminal.get_dynamic_title` applied to a live VTE.
fn dynamic_title(vte: &VteTerminal) -> String {
    let raw = vte.window_title().unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let cwd = vte
        .current_directory_uri()
        .and_then(|uri| glib::filename_from_uri(&uri).ok().map(|(p, _)| p.to_string_lossy().into_owned()));
    title::dynamic_tab_title(&raw, cwd.as_deref(), &home)
}

/// Port of `_setup_static_blur_layer`.
fn setup_static_blur_layer(container: &gtk4::Box) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let base = std::path::PathBuf::from("/");
    let wp = wallpaper::detect_wallpaper_path(&home, &base, wallpaper::real_gsettings_runner);
    if let Some(wp) = wp {
        let pic = Picture::for_filename(&wp);
        pic.set_content_fit(gtk4::ContentFit::Cover);
        pic.set_can_shrink(true);
        pic.set_hexpand(true);
        pic.set_vexpand(true);
        pic.add_css_class("tuxixi-static-blur-pic");
        container.append(&pic);
    } else {
        container.add_css_class("tuxixi-static-blur-fallback");
    }
}

// ---------------------------------------------------------------------
// TabItem (port of TuxixiTabItem)
// ---------------------------------------------------------------------

struct TabItem {
    widget: gtk4::Box,
    label: Label,
    on_select: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_close: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

impl TabItem {
    fn new(title: &str) -> Self {
        let widget = gtk4::Box::new(Orientation::Horizontal, 2);
        widget.add_css_class("tuxixi-tab");

        let label = Label::new(Some(title));
        label.add_css_class("tuxixi-tab-title");
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_max_width_chars(20);
        widget.append(&label);

        let close_btn = Button::with_label("×");
        close_btn.add_css_class("tuxixi-tab-close");
        close_btn.set_can_focus(false);
        close_btn.set_opacity(0.0);
        widget.append(&close_btn);

        let on_select: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_close: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        // Hover reveals the close button.
        let motion = EventControllerMotion::new();
        {
            let close_btn_enter = close_btn.clone();
            motion.connect_enter(move |_, _, _| close_btn_enter.set_opacity(1.0));
            let close_btn_leave = close_btn.clone();
            motion.connect_leave(move |_| close_btn_leave.set_opacity(0.0));
        }
        widget.add_controller(motion);

        // Middle click closes (port of _on_pressed button==2); plain
        // clicks are handled by a click gesture that ignores the close
        // button area.
        let click = GestureClick::new();
        click.set_button(2);
        {
            let on_close = on_close.clone();
            click.connect_pressed(move |gesture, _n, _x, _y| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                fire(&on_close);
            });
        }
        widget.add_controller(click);
        {
            let on_select = on_select.clone();
            let close_btn = close_btn.clone();
            let widget_for_click = widget.clone();
            let gesture = GestureClick::new();
            gesture.set_button(1);
            gesture.connect_pressed(move |gesture, _n, x, y| {
                // Presses on the close button must reach the button itself:
                // claiming the sequence here would cancel its click. So only
                // claim and select when the press is elsewhere on the tab.
                let inside_close = close_btn
                    .compute_bounds(widget_for_click.upcast_ref::<gtk4::Widget>())
                    .map(|rect| {
                        x >= rect.x() as f64
                            && y >= rect.y() as f64
                            && x <= (rect.x() + rect.width()) as f64
                            && y <= (rect.y() + rect.height()) as f64
                    })
                    .unwrap_or(false);
                if inside_close {
                    return;
                }
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                fire(&on_select);
            });
            widget.add_controller(gesture);
        }

        let on_close_btn = on_close.clone();
        close_btn.connect_clicked(move |_| fire(&on_close_btn));

        Self {
            widget,
            label,
            on_select,
            on_close,
        }
    }

    fn set_title(&self, title: &str) {
        self.label.set_text(title);
        self.widget.set_tooltip_text(Some(title));
    }

    fn set_active(&self, active: bool) {
        if active {
            self.widget.add_css_class("active");
        } else {
            self.widget.remove_css_class("active");
        }
    }

    fn set_on_select(&self, f: impl Fn() + 'static) {
        *self.on_select.borrow_mut() = Some(Box::new(f));
    }

    fn set_on_close(&self, f: impl Fn() + 'static) {
        *self.on_close.borrow_mut() = Some(Box::new(f));
    }
}

fn fire(cb: &Rc<RefCell<Option<Box<dyn Fn()>>>>) {
    if let Some(f) = cb.borrow().as_ref() {
        f();
    }
}

// ---------------------------------------------------------------------
// Terminal session + shortcuts (port of TuxixiTerminal)
// ---------------------------------------------------------------------

fn setup_key_controller(vte: &VteTerminal, this: WindowRef) {
    let ctrl = EventControllerKey::new();
    ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let vte_for_keys = vte.clone();
    let settings = this.borrow().settings.clone();
    ctrl.connect_key_pressed(move |_ctrl, keyval, _keycode, state| {
        use gdk::Key;
        let ctrl_mod = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);

        // Alt+1..9 (also keypad) -> select tab.
        if alt && !ctrl_mod && !shift {
            let idx = match keyval {
                Key::_1 | Key::KP_1 => Some(0),
                Key::_2 | Key::KP_2 => Some(1),
                Key::_3 | Key::KP_3 => Some(2),
                Key::_4 | Key::KP_4 => Some(3),
                Key::_5 | Key::KP_5 => Some(4),
                Key::_6 | Key::KP_6 => Some(5),
                Key::_7 | Key::KP_7 => Some(6),
                Key::_8 | Key::KP_8 => Some(7),
                Key::_9 | Key::KP_9 => Some(8),
                _ => None,
            };
            if let Some(idx) = idx {
                TuxixiWindow::select_tab_by_index(&this, idx as i32);
                return glib::Propagation::Stop;
            }
        }

        if !ctrl_mod {
            return glib::Propagation::Proceed;
        }

        // Ctrl+Tab / Ctrl+Shift+Tab -> switch tab.
        if matches!(keyval, Key::Tab | Key::ISO_Left_Tab | Key::KP_Tab) {
            let delta = if shift { -1 } else { 1 };
            TuxixiWindow::switch_tab_relative(&this, delta);
            return glib::Propagation::Stop;
        }
        // Ctrl+PageUp/PageDown -> switch tab (no Shift, like Python).
        if !shift {
            if keyval == Key::Page_Down {
                TuxixiWindow::switch_tab_relative(&this, 1);
                return glib::Propagation::Stop;
            }
            if keyval == Key::Page_Up {
                TuxixiWindow::switch_tab_relative(&this, -1);
                return glib::Propagation::Stop;
            }
        }

        if !shift {
            return glib::Propagation::Proceed;
        }

        match keyval {
            Key::C | Key::c => {
                vte_for_keys.copy_clipboard_format(vte4::Format::Text);
                glib::Propagation::Stop
            }
            Key::V | Key::v => {
                vte_for_keys.paste_clipboard();
                glib::Propagation::Stop
            }
            Key::plus | Key::equal | Key::KP_Add => {
                settings.borrow_mut().change_font_size(1);
                glib::Propagation::Stop
            }
            Key::minus | Key::underscore | Key::KP_Subtract => {
                settings.borrow_mut().change_font_size(-1);
                glib::Propagation::Stop
            }
            Key::T | Key::t => {
                settings.borrow_mut().cycle_theme();
                glib::Propagation::Stop
            }
            Key::O | Key::o => {
                settings.borrow_mut().toggle_transparency();
                glib::Propagation::Stop
            }
            Key::B | Key::b => {
                settings.borrow_mut().cycle_blur_mode();
                glib::Propagation::Stop
            }
            Key::Return | Key::KP_Enter => {
                TuxixiWindow::add_tab(&this);
                glib::Propagation::Stop
            }
            Key::W | Key::w => {
                let active = this.borrow().stack.visible_child();
                let vte_to_close = this
                    .borrow()
                    .tabs
                    .iter()
                    .find(|t| Some(t.widget.clone().upcast::<gtk4::Widget>()) == active)
                    .map(|t| t.vte.clone());
                if let Some(v) = vte_to_close {
                    TuxixiWindow::close_tab(&this, &v);
                }
                glib::Propagation::Stop
            }
            Key::N | Key::n => {
                // New window: activate the application again, which builds
                // another top level window (same as running `tuxixi` twice).
                if let Some(app) = this.borrow().window.application() {
                    if let Some(app) = app.downcast::<Application>().ok() {
                        app.activate();
                    }
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    vte.add_controller(ctrl);
}

/// Port of `_start_session`: feed the welcome banner, then spawn the
/// user's shell with the TUXIXI environment.
fn setup_session(vte: &VteTerminal) {
    let shell_path = shell::default_shell(passwd_shell().as_deref(), Some(&shell_env()));
    let argv_full = shell::shell_argv(&shell_path);
    let argv: Vec<&str> = argv_full.iter().map(|s| s.as_str()).collect();

    // TUXIXI environment + inherited environment, with the TUXIXI bin
    // dir merged into PATH (dedup, appended at the end) so the `ll` and
    // `cls` wrappers installed next to the binary are always reachable
    // inside the session.
    let bin_dir = bin_dir();
    let bin_dir_str = bin_dir.to_string_lossy().into_owned();
    let mut pairs: Vec<(String, String)> = shell::session_env(&bin_dir_str);
    for (k, v) in std::env::vars() {
        if k == "PATH" {
            pairs.push((k, shell::merge_path(&v, &bin_dir_str)));
        } else {
            pairs.push((k, v));
        }
    }
    let envv: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let envv: Vec<&str> = envv.iter().map(|s| s.as_str()).collect();

    let cwd = std::env::var("HOME").unwrap_or_else(|_| "/".into());

    // Welcome banner (port of the tuxixi --welcome feed). Uses the
    // running binary itself, so it works even when `tuxixi` is not on
    // the session PATH yet.
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(out) = std::process::Command::new(exe).arg("--welcome").output() {
            if out.status.success() && !out.stdout.is_empty() {
                let text = String::from_utf8_lossy(&out.stdout).replace('\n', "\r\n");
                vte.feed(text.as_bytes());
            }
        }
    }

    vte.spawn_async(
        vte4::PtyFlags::DEFAULT,
        Some(&cwd),
        &argv,
        &envv,
        glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        None::<&gio::Cancellable>,
        move |result| {
            if let Err(e) = result {
                eprintln!("Error: Shell did not work. - {e}");
            }
        },
    );
}

fn passwd_shell() -> Option<String> {
    // Minimal /etc/passwd lookup for the current uid (port of
    // pwd.getpwuid(os.getuid()).pw_shell).
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

fn shell_env() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

fn bin_dir() -> std::path::PathBuf {
    // The directory that contains the running binary (e.g.
    // ~/.local/bin). The old code appended a literal "bin" component,
    // producing a nonexistent ~/.local/bin/bin path.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

// ---------------------------------------------------------------------
// Application (port of TuxixiApplication)
// ---------------------------------------------------------------------

/// Run the GUI application. Blocks until the last window is closed.
pub fn run() {
    let app = Application::builder().application_id(APP_ID).build();

    let settings: Rc<RefCell<SettingsManager>> =
        Rc::new(RefCell::new(SettingsManager::load_default_location()));

    app.connect_activate(move |app| {
        apply_global_css(&settings);
        let win = TuxixiWindow::build(app, settings.clone());
        win.borrow().window.present();
    });

    app.run();
}

fn apply_global_css(settings: &Rc<RefCell<SettingsManager>>) {
    let provider = CssProvider::new();
    let s = settings.borrow().get().clone();
    let css = crate::css::build_css(&s.theme, s.transparent, s.blur_mode);
    provider.load_from_string(&css);
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    // Live theme switching: reload the SAME provider whenever a setting
    // changes. Reloading invalidates the styles, so Ctrl+Shift+T (theme),
    // Ctrl+Shift+O (glass) and Ctrl+Shift+B (blur) take effect
    // immediately without restarting. Without this listener the CSS was
    // built exactly once at startup and theme changes were invisible.
    settings.borrow_mut().on_change(std::rc::Rc::new(move |s| {
        let css = crate::css::build_css(&s.theme, s.transparent, s.blur_mode);
        provider.load_from_string(&css);
    }));
}
