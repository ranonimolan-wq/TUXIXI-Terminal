//! Glass blur: the compositor-side half of the glass mode.
//!
//! The renderer half (translucent background alpha) lives in
//! `crate::gpu::GlassParams`. This module asks the *compositor* to
//! blur whatever is BEHIND the terminal window, so the desktop really
//! appears blurred through the glass instead of just tinted:
//!
//! * X11: set `_KDE_NET_WM_BLUR_BEHIND_REGION` (CARD32, empty region =
//!   whole window) — honored by KWin, KDE Plasma's Wayland session
//!   (Xwayland windows) and picom's `blur-background` when the
//!   property is respected by the config.
//! * Wayland (native): KWin exposes the same feature through the
//!   `org_kde_kwin_blur` protocol, which would require a full
//!   wayland-client dependency; out of scope here — glass still works
//!   (translucency renders fine), it just skips the blur request.
//! * macOS / Windows: out of scope for this Linux release.
//!
//! Everything is best-effort: the function returns false on any
//! failure and the terminal keeps rendering normally (translucency
//! without blur) — a glass mode that cannot blur must never crash or
//! hang the terminal.

/// Atom used to request compositor blur behind a window.
pub const BLUR_BEHIND_ATOM: &str = "_KDE_NET_WM_BLUR_BEHIND_REGION";

/// Outcome of a blur request, for debug output and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlurOutcome {
    /// The property was set successfully.
    Requested,
    /// No usable X11 display connection (Wayland session, headless CI).
    NoDisplay,
    /// Xlib could not be loaded or the request failed; rendering is
    /// unaffected.
    Unsupported,
}

#[cfg(all(feature = "gui", target_os = "linux"))]
mod imp {
    use super::{BlurOutcome, BLUR_BEHIND_ATOM};

    /// Set the blur-behind property on X11 window `window_id`.
    ///
    /// Opens its own short-lived X connection (XOpenDisplay(NULL)):
    /// the window id is X-server-global, so the property is visible to
    /// the compositor even though the connection is not the one winit
    /// owns. The connection is closed immediately after the sync.
    pub fn request_blur(window_id: u32) -> BlurOutcome {
        let Some(xlib) = load_xlib() else {
            return BlurOutcome::Unsupported;
        };
        unsafe {
            let display = (xlib.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return BlurOutcome::NoDisplay;
            }
            let blur_atom = intern_atom(xlib, display, BLUR_BEHIND_ATOM);
            if blur_atom == 0 {
                (xlib.XCloseDisplay)(display);
                return BlurOutcome::Unsupported;
            }
            let cardinal = intern_atom(xlib, display, "CARDINAL");
            // Empty region (nitems = 0) == blur the whole window.
            (xlib.XChangeProperty)(
                display,
                window_id as u64,
                blur_atom,
                if cardinal == 0 {
                    6 /* XA_CARDINAL fallback */
                } else {
                    cardinal
                },
                32,
                0, /* PropModeReplace */
                std::ptr::null(),
                0,
            );
            (xlib.XSync)(display, 0 /* False */);
            (xlib.XCloseDisplay)(display);
            BlurOutcome::Requested
        }
    }

    /// Remove the blur request again (glass toggled off at runtime).
    /// Best-effort: returns false when Xlib/X11 is not available.
    pub fn remove_blur(window_id: u32) -> bool {
        let Some(xlib) = load_xlib() else {
            return false;
        };
        unsafe {
            let display = (xlib.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return false;
            }
            let blur_atom = intern_atom(xlib, display, BLUR_BEHIND_ATOM);
            if blur_atom != 0 {
                (xlib.XDeleteProperty)(display, window_id as u64, blur_atom);
            }
            (xlib.XSync)(display, 0);
            (xlib.XCloseDisplay)(display);
            blur_atom != 0
        }
    }

    fn load_xlib() -> Option<&'static x11_dl::xlib::Xlib> {
        static CACHE: std::sync::OnceLock<Result<x11_dl::xlib::Xlib, String>> =
            std::sync::OnceLock::new();
        CACHE
            .get_or_init(|| x11_dl::xlib::Xlib::open().map_err(|e| e.to_string()))
            .as_ref()
            .ok()
    }

    fn intern_atom(
        xlib: &x11_dl::xlib::Xlib,
        display: *mut x11_dl::xlib::_XDisplay,
        name: &str,
    ) -> u64 {
        let c = std::ffi::CString::new(name).unwrap();
        unsafe { (xlib.XInternAtom)(display, c.as_ptr(), 0) }
    }
}

#[cfg(all(feature = "gui", target_os = "linux"))]
pub use imp::{remove_blur, request_blur};

/// Non-Linux builds never blur (kept for API symmetry).
#[cfg(all(feature = "gui", not(target_os = "linux")))]
pub fn request_blur(_window_id: u32) -> BlurOutcome {
    BlurOutcome::Unsupported
}

#[cfg(all(feature = "gui", target_os = "linux"))]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_values_are_distinct() {
        assert_ne!(BlurOutcome::Requested, BlurOutcome::NoDisplay);
        assert_ne!(BlurOutcome::Requested, BlurOutcome::Unsupported);
        assert_ne!(BlurOutcome::NoDisplay, BlurOutcome::Unsupported);
    }

    #[test]
    fn atom_name_is_stable() {
        assert_eq!(BLUR_BEHIND_ATOM, "_KDE_NET_WM_BLUR_BEHIND_REGION");
    }

    /// Best-effort in a headless environment: must return WITHOUT
    /// hanging or panicking. NoDisplay (headless CI) or Unsupported
    /// (no libX11) are both fine outcomes; Requested cannot happen
    /// without a display but is not asserted against (a desktop test
    /// run may legitimately succeed).
    #[test]
    fn request_blur_never_panics_headless() {
        let outcome = request_blur(0x1c00001);
        assert!(
            matches!(
                outcome,
                BlurOutcome::Requested | BlurOutcome::NoDisplay | BlurOutcome::Unsupported
            ),
            "unexpected outcome: {outcome:?}"
        );
    }
}
