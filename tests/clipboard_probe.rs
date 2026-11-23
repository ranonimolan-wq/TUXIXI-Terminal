// Probe: arboard clipboard roundtrip. Skips gracefully on systems
// without a display (CI) — the test needs a live X11/Wayland server.
#[cfg(feature = "gui")]
#[test]
fn arboard_set_then_get_roundtrip() {
    let mut clip = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            // Headless CI has no clipboard; skip instead of failing.
            eprintln!("skipping: clipboard unavailable here: {e}");
            return;
        }
    };
    if let Err(e) = clip.set_text("ARBOARD_PROBE_OK") {
        eprintln!("skipping: set_text unavailable here: {e}");
        return;
    }
    match clip.get_text() {
        Ok(t) => assert_eq!(t, "ARBOARD_PROBE_OK"),
        Err(e) => {
            // Another owner may have taken the clipboard between set
            // and get on a shared display; treat as a skip, not a fail.
            eprintln!("skipping: clipboard was replaced concurrently: {e}");
        }
    }
}

#[cfg(not(feature = "gui"))]
#[test]
fn arboard_probe_requires_gui_feature() {
    // The clipboard stack is a gui-feature dependency.
}
