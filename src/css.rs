//! Pure port of the global CSS generation from `main.py`
//! (`TuxixiApplication._update_global_css`). Generating this as a plain
//! string keeps it fully unit-testable without a display server.

use crate::settings::BlurMode;
use crate::theme::{Registry, Theme, SMOKY_ALPHA, TRANSLUCENT_ALPHA};

/// Build the application-wide CSS for the current visual state. Unknown
/// theme names fall back to the default theme (Tokyo Night).
pub fn build_css(theme_name: &str, transparent: bool, blur_mode: BlurMode) -> String {
    let reg = Registry::global();
    let theme = reg
        .get(theme_name)
        .unwrap_or_else(|| reg.fallback())
        .clone();
    build_css_with_theme(&theme, transparent, blur_mode)
}

/// Same, but for an explicit theme (used by tests and callers that hold
/// a [`Theme`] directly, e.g. user themes).
pub fn build_css_with_theme(theme: &Theme, transparent: bool, blur_mode: BlurMode) -> String {
    let bg_hex = theme.background.as_str();
    let fg_hex = theme.foreground.as_str();

    let (win_bg, win_image, tabbar_bg, term_bg, text_hex) = match blur_mode {
        BlurMode::Static => (
            "#0D1117".to_string(),
            "none".to_string(),
            "alpha(#080B10, 0.32)".to_string(),
            "transparent".to_string(),
            "#F8FAFC".to_string(),
        ),
        BlurMode::Smoky => (
            format!("alpha(#2B2936, {SMOKY_ALPHA})"),
            "linear-gradient(135deg, alpha(#433F52, 0.25) 0%, alpha(#2B2936, 0.20) 50%, alpha(#23212C, 0.28) 100%)"
                .to_string(),
            "alpha(#23212C, 0.30)".to_string(),
            "transparent".to_string(),
            "#F8FAFC".to_string(),
        ),
        BlurMode::Off if transparent => (
            format!("alpha(#000000, {TRANSLUCENT_ALPHA})"),
            "none".to_string(),
            "transparent".to_string(),
            "transparent".to_string(),
            "#F8FAFC".to_string(),
        ),
        BlurMode::Off => (
            bg_hex.to_string(),
            "none".to_string(),
            bg_hex.to_string(),
            bg_hex.to_string(),
            fg_hex.to_string(),
        ),
    };

    format!(
        r#"
        window, window.background {{
            background-color: {win_bg};
            background-image: {win_image};
            padding: 0;
            margin: 0;
        }}
        vte-terminal {{
            background-color: {term_bg};
        }}
        scrollbar {{
            background-color: transparent;
        }}
        .tuxixi-static-blur-pic {{
            filter: blur(8px) brightness(0.62) saturate(1.10);
        }}
        .tuxixi-static-blur-fallback {{
            background-image: linear-gradient(135deg, #111827 0%, #1e293b 50%, #0f172a 100%);
        }}
        .tuxixi-tabbar {{
            background-color: {tabbar_bg};
            border-bottom: 1px solid alpha({text_hex}, 0.14);
            padding: 3px 6px;
            min-height: 26px;
        }}
        .tuxixi-tab {{
            background-color: transparent;
            color: alpha({text_hex}, 0.65);
            border-radius: 5px;
            padding: 2px 6px 2px 10px;
            min-height: 22px;
            transition: background-color 120ms ease, color 120ms ease;
        }}
        .tuxixi-tab:hover {{
            background-color: alpha({text_hex}, 0.10);
            color: {text_hex};
        }}
        .tuxixi-tab.active {{
            background-color: alpha({text_hex}, 0.16);
            color: {text_hex};
            font-weight: 600;
        }}
        .tuxixi-tab-title {{
            font-size: 12px;
        }}
        .tuxixi-tab-close {{
            background: transparent;
            border: none;
            box-shadow: none;
            color: alpha({text_hex}, 0.75);
            border-radius: 4px;
            padding: 0;
            margin-left: 4px;
            min-width: 16px;
            min-height: 16px;
            font-size: 12px;
            transition: opacity 120ms ease, background-color 120ms ease, color 120ms ease;
        }}
        .tuxixi-tab-close:hover {{
            background-color: alpha(#f87171, 0.30);
            color: #f87171;
        }}
        .tuxixi-tab-add {{
            background: transparent;
            border: none;
            box-shadow: none;
            color: alpha({text_hex}, 0.70);
            border-radius: 5px;
            padding: 0;
            min-width: 22px;
            min-height: 22px;
            font-size: 14px;
            font-weight: bold;
            transition: background-color 120ms ease, color 120ms ease;
        }}
        .tuxixi-tab-add:hover {{
            background-color: alpha({text_hex}, 0.14);
            color: {text_hex};
        }}
        .tuxixi-blur-badge {{
            font-size: 11px;
            color: alpha({text_hex}, 0.80);
            background-color: alpha({text_hex}, 0.12);
            border-radius: 4px;
            padding: 1px 7px;
            margin-right: 2px;
        }}
        "#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::theme_colors;

    #[test]
    fn normal_mode_uses_theme_colors() {
        let css = build_css("Matrix", false, BlurMode::Off);
        assert!(css.contains("background-color: #001100;"));
        assert!(css.contains("alpha(#00FF00, 0.65)"));
    }

    #[test]
    fn static_blur_uses_fixed_window_background() {
        let css = build_css("Dracula", false, BlurMode::Static);
        assert!(css.contains("#0D1117"));
        assert!(css.contains("alpha(#080B10, 0.32)"));
        assert!(css.contains("background-color: transparent;")); // vte-terminal
        assert!(css.contains("filter: blur(8px) brightness(0.62) saturate(1.10);"));
    }

    #[test]
    fn smoky_blur_uses_smoky_alpha() {
        let css = build_css("Dracula", false, BlurMode::Smoky);
        assert!(css.contains("alpha(#2B2936, 0.87)"));
        assert!(css.contains("linear-gradient(135deg, alpha(#433F52, 0.25) 0%"));
    }

    #[test]
    fn glass_mode_uses_translucent_black() {
        let css = build_css("Dracula", true, BlurMode::Off);
        assert!(css.contains("alpha(#000000, 0.52)"));
        // tabbar becomes transparent in glass mode
        assert!(css.contains("background-color: transparent;\n            border-bottom: 1px solid alpha(#F8FAFC, 0.14);"));
    }

    #[test]
    fn unknown_theme_falls_back_to_tokyo_night() {
        let css = build_css("Bogus Theme", false, BlurMode::Off);
        assert!(css.contains("#1A1B26"));
        assert!(css.contains("#A9B1D6"));
    }

    #[test]
    fn custom_user_theme_colors_are_used() {
        let theme = Theme::from_json(
            r##"{"name": "Custom", "background": "#010203", "foreground": "#040506"}"##,
        )
        .unwrap();
        let css = build_css_with_theme(&theme, false, BlurMode::Off);
        assert!(css.contains("background-color: #010203;"));
        assert!(css.contains("alpha(#040506, 0.65)"));
        // Unknown name lookup still works through the registry.
        assert_eq!(theme_colors("Custom").0, "#1A1B26"); // not registered
    }

    #[test]
    fn css_contains_required_selectors() {
        let css = build_css("Nord", false, BlurMode::Off);
        for selector in [
            "window, window.background {",
            "vte-terminal {",
            "scrollbar {",
            ".tuxixi-tabbar {",
            ".tuxixi-tab {",
            ".tuxixi-tab.active {",
            ".tuxixi-tab-close {",
            ".tuxixi-tab-add {",
            ".tuxixi-blur-badge {",
            ".tuxixi-static-blur-pic {",
            ".tuxixi-static-blur-fallback {",
        ] {
            assert!(css.contains(selector), "missing selector: {selector}");
        }
    }

    #[test]
    fn curly_braces_are_balanced() {
        let css = build_css("Tokyo Night", true, BlurMode::Smoky);
        assert_eq!(
            css.matches('{').count(),
            css.matches('}').count(),
            "unbalanced braces in generated CSS"
        );
    }
}
