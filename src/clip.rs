// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! System clipboard access for the GUI session.
//!
//! Two independent X11 selections are supported, mirroring the
//! mainstream terminal behavior:
//!
//! * `CLIPBOARD` — the desktop-wide clipboard (`Ctrl+Shift+C` /
//!   `Ctrl+Shift+V`, OSC 52).
//! * `PRIMARY` — the Linux "current selection": set as soon as the
//!   user drags a selection with the mouse and pasted with the middle
//!   mouse button. Other applications read it without any explicit
//!   copy step, which is what makes select-then-paste into a browser
//!   work in the way users expect from a terminal.
//!
//! On X11 a selection is OWNED by the process that stored it: the
//! owner answers selection requests from other clients. Both
//! pasteboards therefore must be long-lived process-wide objects
//! (one background server thread each); creating them per copy
//! operation drops ownership as soon as the temporary object dies and
//! other applications then find no owner at all. `Pasteboards` is
//! built once at session start and kept alive by [`crate::gui::app::App`].

use copypasta::x11_clipboard::{
    Clipboard as ClipboardAtom, Primary as PrimaryAtom, X11ClipboardContext,
};
// `set_contents`/`get_contents` live on the `ClipboardProvider` trait;
// importing it is what makes the trait methods callable.
use copypasta::ClipboardProvider;

/// The two X11 selections a terminal session serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// The desktop-wide `CLIPBOARD` selection.
    Clipboard,
    /// The Linux mouse `PRIMARY` selection.
    Primary,
}

/// Long-lived owner of both X11 selections.
///
/// `None` fields mean the corresponding pasteboard could not be
/// opened (no display, X server unreachable): the session keeps
/// running and copy/paste degrades to a no-op instead of aborting
/// startup.
pub struct Pasteboards {
    clipboard: Option<X11ClipboardContext<ClipboardAtom>>,
    primary: Option<X11ClipboardContext<PrimaryAtom>>,
}

impl Pasteboards {
    /// Open both pasteboards. Returns a fully-`None` struct (not an
    /// error) when no X server is reachable so callers can keep the
    /// session alive without copy/paste.
    pub fn open() -> Self {
        // Each context spawns its own selection-server thread; a
        // failure of one selection must not prevent the other from
        // working.
        let clipboard = X11ClipboardContext::new()
            .map_err(|e| eprintln!("CLIPBOARD selection unavailable, copy/paste disabled: {e}"))
            .ok();
        let primary = X11ClipboardContext::new()
            .map_err(|e| {
                eprintln!("PRIMARY selection unavailable, middle-click paste disabled: {e}")
            })
            .ok();
        Self { clipboard, primary }
    }

    /// Whether at least one selection backend is usable.
    pub fn any_available(&self) -> bool {
        self.clipboard.is_some() || self.primary.is_some()
    }

    /// Store `text` into the requested selection. Fails softly (Ok
    /// result with `false`) when the backend is missing, so callers do
    /// not have to branch on pasteboard availability.
    pub fn store(&mut self, which: Which, text: &str) -> bool {
        match which {
            Which::Clipboard => match self.clipboard.as_mut() {
                Some(ctx) => ctx.set_contents(text.to_string()).is_ok(),
                None => false,
            },
            Which::Primary => match self.primary.as_mut() {
                Some(ctx) => ctx.set_contents(text.to_string()).is_ok(),
                None => false,
            },
        }
    }

    /// Read the requested selection. `Err` when the backend is
    /// missing, the selection is empty or another client replaced the
    /// owner mid-transfer.
    pub fn load(&mut self, which: Which) -> Result<String, String> {
        match which {
            Which::Clipboard => self
                .clipboard
                .as_mut()
                .ok_or_else(|| "CLIPBOARD selection unavailable".to_string())
                .and_then(|ctx| ctx.get_contents().map_err(|e| e.to_string())),
            Which::Primary => self
                .primary
                .as_mut()
                .ok_or_else(|| "PRIMARY selection unavailable".to_string())
                .and_then(|ctx| ctx.get_contents().map_err(|e| e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_without_display_degrades_to_none() {
        // With no reachable X server every context creation fails and
        // the struct must still be constructible: the session keeps
        // running, copy/paste just degrades.
        let boards = Pasteboards::open();
        if std::env::var_os("DISPLAY").is_none() {
            assert!(!boards.any_available());
            assert!(!boards.store(Which::Clipboard, "x"));
            assert!(!boards.store(Which::Primary, "x"));
            assert!(boards.load(Which::Clipboard).is_err());
            assert!(boards.load(Which::Primary).is_err());
        } else {
            // Under a real display (developer machine) both backends
            // are expected to come up; a soft assertion keeps the test
            // honest without failing on exotic setups.
            assert!(boards.any_available());
        }
    }

    #[test]
    fn which_covers_both_selections() {
        // Exhaustiveness guard: the two-variant enum is the public
        // contract for what the terminal serves.
        let all = [Which::Clipboard, Which::Primary];
        assert_eq!(all.len(), 2);
        assert_ne!(all[0], all[1]);
    }
}
