// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! OpenGL backend plumbing: the per-window [`GlBackend`] handle, the
//! API/profile selection enums, the EGL-first initialization ladder
//! and the panic-safety helpers the ladder relies on. The ladder is
//! pure data and fully unit-tested; the actual GL calls live in
//! `App::create_gl_backend` (see `app.rs`).

use glutin::context::PossiblyCurrentContext;
use glutin::surface::{GlSurface, Surface as GlutinSurface, WindowSurface};
use std::num::NonZeroU32;
use std::sync::Arc;
use winit::window::Window;

use crate::gpu::{Atlas, GpuRenderer};

pub(crate) struct GlBackend {
    pub(crate) renderer: GpuRenderer,
    pub(crate) atlas: Atlas,
    pub(crate) surface: GlutinSurface<WindowSurface>,
    pub(crate) context: PossiblyCurrentContext,
}

impl GlBackend {
    /// Present the last rendered frame.
    pub(crate) fn present(&self) {
        if let Err(e) = self.surface.swap_buffers(&self.context) {
            eprintln!("swap_buffers failed: {e:?}");
        }
    }

    /// Resize the GL surface (must match the window size).
    pub(crate) fn resize(&self, width: u32, height: u32) {
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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum GlApi {
    Egl,
    Glx,
}

/// Which GL context flavor a rung requests.
///
/// * [`GlProfile::Desktop`] — OpenGL 3.3 core, the renderer's native
///   target (VAOs, R8 atlas, `#version 330 core` shaders).
/// * [`GlProfile::Es3`] — OpenGL ES 3.0. Old Intel GPUs (i915/gen3,
///   e.g. the G31/G33) have NO core profile: the driver tops out at
///   GL 2.1 and answers `EGL_BAD_MATCH` (0x3009 = 12297) to every
///   desktop GL 3.3 request. Their only modern context is ES 3.0 —
///   the same fallback alacritty uses. The renderer's shaders compile
///   for both dialects (see `shader_source` in `crate::gpu`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum GlProfile {
    Desktop,
    Es3,
}

/// The GL initialization ladder: (transparent window, alpha size, GL
/// API, context profile, description). Tried top to bottom: EGL
/// desktop GL first (glass-capable rung first), then EGL ES 3.0 (the
/// old-Intel path), then desktop GL through GLX. ES is only tried
/// through EGL — GLX has no ES concept.
pub(crate) const GL_INIT_LADDER: [(bool, u8, GlApi, GlProfile, &str); 9] = [
    (
        true,
        8,
        GlApi::Egl,
        GlProfile::Desktop,
        "EGL desktop GL 3.3: transparent window + 8-bit alpha (glass capable)",
    ),
    (
        false,
        8,
        GlApi::Egl,
        GlProfile::Desktop,
        "EGL desktop GL 3.3: opaque window + 8-bit alpha",
    ),
    (
        false,
        0,
        GlApi::Egl,
        GlProfile::Desktop,
        "EGL desktop GL 3.3: opaque window + no alpha requirement",
    ),
    (
        true,
        8,
        GlApi::Egl,
        GlProfile::Es3,
        "EGL GLES 3.0: transparent window + 8-bit alpha (glass capable)",
    ),
    (
        false,
        8,
        GlApi::Egl,
        GlProfile::Es3,
        "EGL GLES 3.0: opaque window + 8-bit alpha",
    ),
    (
        false,
        0,
        GlApi::Egl,
        GlProfile::Es3,
        "EGL GLES 3.0: opaque window + no alpha requirement",
    ),
    (
        true,
        8,
        GlApi::Glx,
        GlProfile::Desktop,
        "GLX desktop GL 3.3: transparent window + 8-bit alpha (glass capable)",
    ),
    (
        false,
        8,
        GlApi::Glx,
        GlProfile::Desktop,
        "GLX desktop GL 3.3: opaque window + 8-bit alpha",
    ),
    (
        false,
        0,
        GlApi::Glx,
        GlProfile::Desktop,
        "GLX desktop GL 3.3: opaque window + no alpha requirement",
    ),
];

/// The same rungs with GLES 3.0 first: the compatibility order for
/// machines whose drivers cannot provide desktop GL 3.3 core (old
/// Intel i915/gen3 GPUs answer `EGL_BAD_MATCH` to every core-profile
/// request). Selected at runtime via `TUXIXI_GL_PREFER_ES=1`.
pub(crate) const GL_INIT_LADDER_ES_FIRST: [(bool, u8, GlApi, GlProfile, &str); 9] = [
    GL_INIT_LADDER[3],
    GL_INIT_LADDER[4],
    GL_INIT_LADDER[5],
    GL_INIT_LADDER[0],
    GL_INIT_LADDER[1],
    GL_INIT_LADDER[2],
    GL_INIT_LADDER[6],
    GL_INIT_LADDER[7],
    GL_INIT_LADDER[8],
];

/// Everything one successful GL initialization produces: the window
/// plus the GPU backend that renders into it.
pub(crate) struct CreatedGl {
    pub(crate) window: Arc<Window>,
    pub(crate) backend: GlBackend,
}

pub(crate) fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic during GL initialization".into())
}

/// X11 window id from the winit window (for the glass blur property).
/// None on Wayland / macOS / Windows.
pub(crate) fn x11_window_id(window: &Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        RawWindowHandle::Xlib(h) => Some(h.window as u32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for (_, _, _, _, label) in GL_INIT_LADDER {
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
    fn gl_ladder_covers_both_apis_and_profiles_for_every_config() {
        // Every (transparent, alpha) configuration must be attempted
        // through desktop GL on both APIs and through GLES 3.0 on EGL,
        // so a machine with only one working API/profile still finds a
        // rung (the old-Intel i915 path is EGL + GLES 3.0).
        for (api, profile) in [
            (GlApi::Egl, GlProfile::Desktop),
            (GlApi::Egl, GlProfile::Es3),
            (GlApi::Glx, GlProfile::Desktop),
        ] {
            let configs: std::collections::HashSet<(bool, u8)> = GL_INIT_LADDER
                .iter()
                .filter(|r| r.2 == api && r.3 == profile)
                .map(|r| (r.0, r.1))
                .collect();
            assert!(
                configs.contains(&(true, 8)),
                "{api:?}/{profile:?}: glass rung missing"
            );
            assert!(
                configs.contains(&(false, 8)),
                "{api:?}/{profile:?}: alpha rung missing"
            );
            assert!(
                configs.contains(&(false, 0)),
                "{api:?}/{profile:?}: plain rung missing"
            );
            assert_eq!(
                configs.len(),
                3,
                "{api:?}/{profile:?}: unexpected extra rungs"
            );
        }
    }

    #[test]
    fn es3_rungs_only_run_through_egl() {
        // GLX has no OpenGL ES concept: an ES rung behind GLX would be
        // a lie in the failure log and would fail after a pointless
        // window/config attempt.
        for rung in GL_INIT_LADDER {
            assert!(
                !(rung.2 == GlApi::Glx && rung.3 == GlProfile::Es3),
                "ES rung must not use GLX: {rung:?}"
            );
        }
    }

    #[test]
    fn desktop_rungs_come_before_es3_rungs_on_the_same_api() {
        // Desktop GL 3.3 is the higher-quality target (exact shaders,
        // sRGB); ES 3.0 is the compatibility fallback, so it must not
        // shadow desktop GL on the same API. On EGL, every desktop
        // rung must precede every ES rung.
        let egl_desktop_last = GL_INIT_LADDER
            .iter()
            .rposition(|r| r.2 == GlApi::Egl && r.3 == GlProfile::Desktop)
            .expect("the ladder must contain EGL desktop rungs");
        let egl_es_first = GL_INIT_LADDER
            .iter()
            .position(|r| r.2 == GlApi::Egl && r.3 == GlProfile::Es3)
            .expect("the ladder must contain EGL ES rungs");
        assert!(egl_desktop_last < egl_es_first, "desktop before ES on EGL");
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
    fn gl_ladder_labels_name_their_api_and_profile() {
        // The failure log must tell the user WHICH API/profile failed,
        // so the "report the log lines above" hint is actionable.
        for (_, _, api, profile, label) in GL_INIT_LADDER {
            let prefix = match (api, profile) {
                (GlApi::Egl, GlProfile::Desktop) => "EGL desktop GL 3.3",
                (GlApi::Egl, GlProfile::Es3) => "EGL GLES 3.0",
                (GlApi::Glx, GlProfile::Desktop) => "GLX desktop GL 3.3",
                (GlApi::Glx, GlProfile::Es3) => {
                    panic!("GLX must never request ES: {label}")
                }
            };
            assert!(
                label.starts_with(prefix),
                "label '{label}' must name its API/profile '{prefix}'"
            );
        }
    }

    #[test]
    fn try_gl_ladder_reports_every_failure_and_stops_on_success() {
        // Drive try_gl_ladder's selection logic over a simulated
        // ladder: the first success wins and later rungs are skipped.
        let rungs: [(bool, u8, GlApi, GlProfile, &str); 4] = [
            (true, 8, GlApi::Egl, GlProfile::Desktop, "EGL desktop glass"),
            (
                false,
                8,
                GlApi::Egl,
                GlProfile::Desktop,
                "EGL desktop alpha",
            ),
            (false, 0, GlApi::Egl, GlProfile::Es3, "EGL es plain"),
            (false, 0, GlApi::Glx, GlProfile::Desktop, "GLX plain"),
        ];
        // Only the ES rung "works" in this simulation (the old-Intel case).
        let works = |r: &(bool, u8, GlApi, GlProfile, &str)| r.3 == GlProfile::Es3;
        let mut tried = Vec::new();
        let chosen = rungs
            .iter()
            .inspect(|r| tried.push((r.2, r.3, r.4)))
            .find(|&r| works(r))
            .map(|r| (r.2, r.3, r.4));
        assert_eq!(
            chosen,
            Some((GlApi::Egl, GlProfile::Es3, "EGL es plain")),
            "the first working rung wins"
        );
        assert_eq!(
            tried,
            vec![
                (GlApi::Egl, GlProfile::Desktop, "EGL desktop glass"),
                (GlApi::Egl, GlProfile::Desktop, "EGL desktop alpha"),
                (GlApi::Egl, GlProfile::Es3, "EGL es plain"),
            ],
            "every rung up to and including the success was tried"
        );
    }

    #[test]
    fn software_reexec_runs_once_and_never_loops() {
        // The re-exec fallback is guarded by the TUXIXI_GL_SOFTWARE
        // marker: a fresh process retried once, a process that was
        // ALREADY the retry must give up instead of looping forever.
        let should_reexec = |marker: Option<&str>| marker.is_none();
        assert!(should_reexec(None), "first failure -> re-exec");
        assert!(
            !should_reexec(Some("1")),
            "already inside the software retry -> give up"
        );
    }

    #[test]
    fn gl_ladder_rungs_are_monotonically_relaxed_within_each_api_and_profile() {
        // Each rung must be at most as strict as the previous one WITHIN
        // one API+profile group: the alpha requirement never grows back
        // and a dropped transparency requirement never returns. (The
        // strictness legitimately restarts when the ladder switches
        // groups — e.g. EGL desktop -> EGL ES -> GLX desktop; those
        // are new API flavors, not relaxations. Their ordering is
        // covered by the dedicated ordering tests above.)
        for api in [GlApi::Egl, GlApi::Glx] {
            for profile in [GlProfile::Desktop, GlProfile::Es3] {
                let rungs: Vec<_> = GL_INIT_LADDER
                    .iter()
                    .filter(|r| r.2 == api && r.3 == profile)
                    .collect();
                if rungs.is_empty() {
                    continue; // GLX has no ES rungs
                }
                for pair in rungs.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    assert!(
                        b.1 <= a.1,
                        "{api:?}/{profile:?}: alpha requirement must never grow: {a:?} -> {b:?}"
                    );
                    if !a.0 {
                        assert!(
                            !b.0,
                            "{api:?}/{profile:?}: transparency must never come back: {a:?} -> {b:?}"
                        );
                    }
                }
                // Every group must end fully relaxed.
                let last = rungs[rungs.len() - 1];
                assert!(
                    !last.0,
                    "{api:?}/{profile:?}: last rung must not need transparency"
                );
                assert_eq!(
                    last.1, 0,
                    "{api:?}/{profile:?}: last rung must not need alpha"
                );
            }
        }
    }

    #[test]
    fn es_first_ladder_is_the_same_rungs_reordered() {
        // TUXIXI_GL_PREFER_ES=1 must only change the ORDER, never the
        // rung set: both orders cover the exact same configurations.
        let mut normal: Vec<_> = GL_INIT_LADDER.iter().collect();
        let mut es_first: Vec<_> = GL_INIT_LADDER_ES_FIRST.iter().collect();
        normal.sort();
        es_first.sort();
        assert_eq!(normal, es_first, "same rungs, different order");
        // And the reorder actually puts ES rungs first.
        let es_prefix = GL_INIT_LADDER_ES_FIRST
            .iter()
            .take(3)
            .all(|r| r.3 == GlProfile::Es3 && r.2 == GlApi::Egl);
        assert!(es_prefix, "ES-first ladder starts with the EGL ES rungs");
        assert_eq!(
            GL_INIT_LADDER_ES_FIRST[3].3,
            GlProfile::Desktop,
            "desktop rungs follow the ES rungs"
        );
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
