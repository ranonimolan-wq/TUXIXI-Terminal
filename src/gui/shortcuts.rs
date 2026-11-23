// SPDX-License-Identifier: GPL-3.0
// Copyright (c) 2026 ranonimolan-wq / TUXIXI. All rights reserved.

//! The Ctrl+Shift GUI shortcuts (copy / paste / cycle theme /
//! toggle glass). Resolution is a pure, unit-tested mapping so the
//! GUI only consumes the resulting action.

use crate::input::Key as InputKey;

/// The Ctrl+Shift GUI shortcuts, resolved from a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Shortcut {
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
pub(crate) fn shortcut_from_key(key: &InputKey) -> Option<Shortcut> {
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
}
