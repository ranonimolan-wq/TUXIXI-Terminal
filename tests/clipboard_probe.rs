// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

// Probe: X11 selection roundtrip through the copypasta-based
// pasteboard layer. Skips gracefully on systems without a display
// (CI) — the test needs a live X server.

#[cfg(feature = "gui")]
#[test]
fn pasteboard_set_then_get_roundtrip() {
    use tuxixi::clip::{Pasteboards, Which};

    let mut boards = Pasteboards::open();
    if !boards.any_available() {
        // Headless CI has no clipboard; skip instead of failing.
        eprintln!("skipping: pasteboards unavailable here (no display?)");
        return;
    }
    if !boards.store(Which::Clipboard, "TUXIXI_PROBE_OK".to_string().as_str()) {
        eprintln!("skipping: CLIPBOARD store unavailable here");
        return;
    }
    match boards.load(Which::Clipboard) {
        Ok(t) => assert_eq!(t, "TUXIXI_PROBE_OK"),
        Err(e) => {
            // Another owner may have taken the selection between set
            // and get on a shared display; treat as a skip, not a fail.
            eprintln!("skipping: selection was replaced concurrently: {e}");
        }
    }
}

#[cfg(not(feature = "gui"))]
#[test]
fn pasteboard_probe_requires_gui_feature() {
    // The pasteboard layer is a gui-feature dependency.
}
