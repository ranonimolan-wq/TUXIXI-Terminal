//! Keyboard input encoding: maps abstract key events (produced by the
//! winit GUI from `winit::event::KeyEvent`) into the byte sequences a
//! VT client expects.
//!
//! This module is intentionally GUI-free: it is a pure function set
//! over our own [`Key`] / [`Modifiers`] types, so every encoding rule
//! is unit-testable without a display.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl Modifiers {
    /// xterm modifier parameter: 1 + 1(shift) + 2(alt) + 4(ctrl).
    pub fn xterm_code(&self) -> u16 {
        let mut code = 1;
        if self.shift {
            code += 1;
        }
        if self.alt {
            code += 2;
        }
        if self.ctrl {
            code += 4;
        }
        code as u16
    }

    pub fn has_mods(&self) -> bool {
        self.ctrl || self.alt || self.shift
    }
}

/// Encode a key press. Returns `None` when the key produces no bytes
/// (e.g. logo-modified combos, unknown keys) so the GUI can fall back
/// to its own shortcuts.
pub fn encode(key: Key, mods: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    if mods.logo {
        return None; // never swallow OS shortcuts
    }

    let csi = |final_byte: char| -> Vec<u8> { format!("\x1b[{final_byte}").into_bytes() };
    let mod_csi = |n: u16, final_byte: char| -> Vec<u8> {
        format!("\x1b[{};{}{}", n, mods.xterm_code(), final_byte).into_bytes()
    };

    match key {
        Key::Char(c) => encode_char(c, mods),
        Key::Enter => {
            if mods.ctrl {
                Some(b"\x0a".to_vec()) // C-j
            } else if mods.alt {
                Some(b"\x1b\r".to_vec())
            } else if mods.shift {
                None // shift+enter is a GUI shortcut candidate
            } else {
                Some(b"\r".to_vec())
            }
        }
        Key::Backspace => {
            if mods.alt {
                Some(b"\x1b\x7f".to_vec())
            } else if mods.ctrl {
                Some(b"\x08".to_vec())
            } else {
                Some(b"\x7f".to_vec())
            }
        }
        Key::Tab => {
            if mods.shift {
                Some(csi('Z'))
            } else {
                Some(b"\t".to_vec())
            }
        }
        Key::Escape => {
            if mods.alt {
                Some(b"\x1b\x1b".to_vec())
            } else {
                Some(b"\x1b".to_vec())
            }
        }
        Key::Up | Key::Down | Key::Left | Key::Right => {
            let f = match key {
                Key::Up => 'A',
                Key::Down => 'B',
                Key::Right => 'C',
                _ => 'D',
            };
            if mods.has_mods() {
                Some(mod_csi(1, f))
            } else if app_cursor {
                Some(format!("\x1bO{f}").into_bytes())
            } else {
                Some(csi(f))
            }
        }
        Key::Home | Key::End => {
            let f = if key == Key::Home { 'H' } else { 'F' };
            if mods.has_mods() {
                Some(mod_csi(1, f))
            } else if app_cursor {
                Some(format!("\x1bO{f}").into_bytes())
            } else {
                Some(csi(f))
            }
        }
        Key::Insert => Some(num_csi(&mods, 2)),
        Key::Delete => Some(num_csi(&mods, 3)),
        Key::PageUp => Some(num_csi(&mods, 5)),
        Key::PageDown => Some(num_csi(&mods, 6)),
        Key::F(n) => encode_function_key(n, &mods),
    }
}

fn num_csi(mods: &Modifiers, n: u16) -> Vec<u8> {
    if mods.has_mods() {
        format!("\x1b[{};{}~", n, mods.xterm_code()).into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

fn encode_function_key(n: u8, mods: &Modifiers) -> Option<Vec<u8>> {
    // F1-F4: ESC O P/Q/R/S (modifier variants use the CSI form).
    if (1..=4).contains(&n) {
        let f = match n {
            1 => 'P',
            2 => 'Q',
            3 => 'R',
            _ => 'S',
        };
        if mods.has_mods() {
            return Some(format!("\x1b[{};{}{}", 1 + n, mods.xterm_code(), f).into_bytes());
        }
        return Some(format!("\x1bO{f}").into_bytes());
    }
    // F5-F12: numbered tilde sequences (xterm codes).
    let code = match n {
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        12 => 24,
        _ => return None,
    };
    Some(num_csi(mods, code))
}

fn encode_char(c: char, mods: Modifiers) -> Option<Vec<u8>> {
    // Control characters: ctrl+letter / ctrl+space / ctrl+@ etc.
    if mods.ctrl && !mods.alt {
        if let Some(b) = ctrl_byte(c, mods.shift) {
            return Some(vec![b]);
        }
        // ctrl + non-ASCII: fall through to text (rare, harmless).
        return Some(c.to_string().into_bytes());
    }
    if mods.alt {
        let mut out = vec![0x1b];
        out.extend(c.to_string().into_bytes());
        return Some(out);
    }
    Some(c.to_string().into_bytes())
}

/// ctrl+<c> byte per the classic mapping. `shift` enables the
/// ctrl+shift letter forms (= same as plain ctrl on most terminals).
fn ctrl_byte(c: char, _shift: bool) -> Option<u8> {
    let lower = c.to_ascii_lowercase();
    match c {
        '@' => Some(0x00),
        ' ' => Some(0x00),
        'a'..='z' => Some(lower as u8 - b'a' + 1),
        'A'..='Z' => Some(lower as u8 - b'a' + 1),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '/' => Some(0x1f),
        '2' => Some(0x00),
        '3' => Some(0x1b),
        '4' => Some(0x1c),
        '5' => Some(0x1d),
        '6' => Some(0x1e),
        '7' => Some(0x1f),
        '8' => Some(0x7f),
        '?' => Some(0x7f),
        _ => None,
    }
}

/// Encode a pasted string, wrapping it in bracketed paste markers when
/// the application requested them (DECSET 2004).
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend(text.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        // Strip CR from CRLF pairs like real terminals do on paste.
        text.replace("\r\n", "\n").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        ctrl: false,
        alt: false,
        shift: false,
        logo: false,
    };

    fn mods(ctrl: bool, alt: bool, shift: bool) -> Modifiers {
        Modifiers {
            ctrl,
            alt,
            shift,
            logo: false,
        }
    }

    fn bytes(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    #[test]
    fn plain_text_passes_through_as_utf8() {
        assert_eq!(encode(Key::Char('a'), NONE, false), Some(bytes("a")));
        assert_eq!(encode(Key::Char('Z'), NONE, false), Some(bytes("Z")));
        assert_eq!(
            encode(Key::Char('ş'), NONE, false),
            Some("ş".as_bytes().to_vec())
        );
    }

    #[test]
    fn control_keys_map_to_the_classic_bytes() {
        assert_eq!(encode(Key::Enter, NONE, false), Some(b"\r".to_vec()));
        assert_eq!(encode(Key::Backspace, NONE, false), Some(b"\x7f".to_vec()));
        assert_eq!(encode(Key::Tab, NONE, false), Some(b"\t".to_vec()));
        assert_eq!(encode(Key::Escape, NONE, false), Some(b"\x1b".to_vec()));
    }

    #[test]
    fn ctrl_letter_combos_produce_control_codes() {
        assert_eq!(
            encode(Key::Char('c'), mods(true, false, false), false),
            Some(vec![0x03])
        );
        assert_eq!(
            encode(Key::Char('u'), mods(true, false, false), false),
            Some(vec![0x15])
        );
        assert_eq!(
            encode(Key::Char('C'), mods(true, false, false), false),
            Some(vec![0x03])
        );
    }

    #[test]
    fn ctrl_special_keys_cover_the_full_set() {
        assert_eq!(
            encode(Key::Char(' '), mods(true, false, false), false),
            Some(vec![0x00])
        );
        assert_eq!(
            encode(Key::Char('['), mods(true, false, false), false),
            Some(vec![0x1b])
        );
        assert_eq!(
            encode(Key::Char('_'), mods(true, false, false), false),
            Some(vec![0x1f])
        );
        assert_eq!(
            encode(Key::Char('8'), mods(true, false, false), false),
            Some(vec![0x7f])
        );
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(
            encode(Key::Char('b'), mods(false, true, false), false),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(
            encode(Key::Char('f'), mods(false, true, false), false),
            Some(b"\x1bf".to_vec())
        );
        assert_eq!(
            encode(Key::Backspace, mods(false, true, false), false),
            Some(b"\x1b\x7f".to_vec())
        );
        assert_eq!(
            encode(Key::Escape, mods(false, true, false), false),
            Some(b"\x1b\x1b".to_vec())
        );
        assert_eq!(
            encode(Key::Enter, mods(false, true, false), false),
            Some(b"\x1b\r".to_vec())
        );
    }

    #[test]
    fn arrows_use_csi_or_ss3_depending_on_app_cursor_mode() {
        for (key, f) in [
            (Key::Up, 'A'),
            (Key::Down, 'B'),
            (Key::Right, 'C'),
            (Key::Left, 'D'),
        ] {
            assert_eq!(
                encode(key, NONE, false),
                Some(format!("\x1b[{f}").into_bytes())
            );
            assert_eq!(
                encode(key, NONE, true),
                Some(format!("\x1bO{f}").into_bytes())
            );
        }
    }

    #[test]
    fn modified_arrows_use_the_xterm_parameter_form() {
        // ctrl+right -> ESC[1;5C
        assert_eq!(
            encode(Key::Right, mods(true, false, false), false),
            Some(b"\x1b[1;5C".to_vec())
        );
        // shift+up -> ESC[1;2A
        assert_eq!(
            encode(Key::Up, mods(false, false, true), true),
            Some(b"\x1b[1;2A".to_vec())
        );
        // alt+down -> ESC[1;3B
        assert_eq!(
            encode(Key::Down, mods(false, true, false), false),
            Some(b"\x1b[1;3B".to_vec())
        );
    }

    #[test]
    fn home_end_pageup_pagedown_insert_delete_use_tilde_form() {
        assert_eq!(encode(Key::Home, NONE, false), Some(b"\x1b[H".to_vec()));
        assert_eq!(encode(Key::End, NONE, false), Some(b"\x1b[F".to_vec()));
        assert_eq!(encode(Key::Home, NONE, true), Some(b"\x1bOH".to_vec()));
        assert_eq!(encode(Key::End, NONE, true), Some(b"\x1bOF".to_vec()));
        assert_eq!(encode(Key::Insert, NONE, false), Some(b"\x1b[2~".to_vec()));
        assert_eq!(encode(Key::Delete, NONE, false), Some(b"\x1b[3~".to_vec()));
        assert_eq!(encode(Key::PageUp, NONE, false), Some(b"\x1b[5~".to_vec()));
        assert_eq!(
            encode(Key::PageDown, NONE, false),
            Some(b"\x1b[6~".to_vec())
        );
        assert_eq!(
            encode(Key::Delete, mods(true, false, false), false),
            Some(b"\x1b[3;5~".to_vec())
        );
    }

    #[test]
    fn function_keys_use_ss3_and_tilde_codes() {
        assert_eq!(encode(Key::F(1), NONE, false), Some(b"\x1bOP".to_vec()));
        assert_eq!(encode(Key::F(2), NONE, false), Some(b"\x1bOQ".to_vec()));
        assert_eq!(encode(Key::F(3), NONE, false), Some(b"\x1bOR".to_vec()));
        assert_eq!(encode(Key::F(4), NONE, false), Some(b"\x1bOS".to_vec()));
        assert_eq!(encode(Key::F(5), NONE, false), Some(b"\x1b[15~".to_vec()));
        assert_eq!(encode(Key::F(10), NONE, false), Some(b"\x1b[21~".to_vec()));
        assert_eq!(encode(Key::F(11), NONE, false), Some(b"\x1b[23~".to_vec()));
        assert_eq!(encode(Key::F(12), NONE, false), Some(b"\x1b[24~".to_vec()));
        assert_eq!(
            encode(Key::F(5), mods(true, false, false), false),
            Some(b"\x1b[15;5~".to_vec())
        );
        assert_eq!(encode(Key::F(13), NONE, false), None);
    }

    #[test]
    fn shift_tab_sends_backtab() {
        assert_eq!(
            encode(Key::Tab, mods(false, false, true), false),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn ctrl_enter_is_linefeed() {
        assert_eq!(
            encode(Key::Enter, mods(true, false, false), false),
            Some(b"\x0a".to_vec())
        );
    }

    #[test]
    fn logo_modifier_never_swallows_os_shortcuts() {
        for key in [Key::Char('t'), Key::Up, Key::Tab, Key::Enter, Key::Escape] {
            let m = Modifiers {
                logo: true,
                ctrl: false,
                alt: false,
                shift: false,
            };
            assert_eq!(encode(key, m, false), None);
        }
    }

    #[test]
    fn xterm_modifier_codes_match_the_spec() {
        assert_eq!(NONE.xterm_code(), 1);
        assert_eq!(mods(false, false, true).xterm_code(), 2);
        assert_eq!(mods(false, true, false).xterm_code(), 3);
        assert_eq!(mods(true, false, false).xterm_code(), 5);
        assert_eq!(mods(true, true, true).xterm_code(), 8);
    }

    #[test]
    fn paste_wraps_in_brackets_when_requested() {
        let out = encode_paste("hello", true);
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b[200~hello\x1b[201~");
    }

    #[test]
    fn paste_without_brackets_strips_crlf() {
        let out = encode_paste("a\r\nb\r\n", false);
        assert_eq!(String::from_utf8(out).unwrap(), "a\nb\n");
        // With brackets the raw text is preserved verbatim.
        let out = encode_paste("a\r\nb", true);
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b[200~a\r\nb\x1b[201~");
    }

    #[test]
    fn unicode_paste_survives_byte_encoding() {
        let out = encode_paste("漢字 utf-8 ✓", false);
        assert_eq!(String::from_utf8(out).unwrap(), "漢字 utf-8 ✓");
    }

    #[test]
    fn ctrl_non_ascii_falls_back_to_text() {
        assert_eq!(
            encode(Key::Char('ö'), mods(true, false, false), false),
            Some("ö".as_bytes().to_vec())
        );
    }
}
