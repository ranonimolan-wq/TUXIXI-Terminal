//! Port of `terminal.py` key handling helpers: the Alt+Number tab
//! selection map and the Ctrl+Tab direction rule.
//!
//! Key constants mirror GDK key values so the same logic can be unit
//! tested without a display server.

pub const KEY_1: u32 = 0x031;
pub const KEY_2: u32 = 0x032;
pub const KEY_3: u32 = 0x033;
pub const KEY_4: u32 = 0x034;
pub const KEY_5: u32 = 0x035;
pub const KEY_6: u32 = 0x036;
pub const KEY_7: u32 = 0x037;
pub const KEY_8: u32 = 0x038;
pub const KEY_9: u32 = 0x039;

pub const KEY_KP_1: u32 = 0xFFB1;
pub const KEY_KP_2: u32 = 0xFFB2;
pub const KEY_KP_3: u32 = 0xFFB3;
pub const KEY_KP_4: u32 = 0xFFB4;
pub const KEY_KP_5: u32 = 0xFFB5;
pub const KEY_KP_6: u32 = 0xFFB6;
pub const KEY_KP_7: u32 = 0xFFB7;
pub const KEY_KP_8: u32 = 0xFFB8;
pub const KEY_KP_9: u32 = 0xFFB9;

/// Port of `ALT_NUMBER_KEYS`: Alt+1..9 / Alt+KP_1..9 select tab 0..8.
pub fn alt_number_tab_index(keyval: u32) -> Option<usize> {
    match keyval {
        KEY_1 | KEY_KP_1 => Some(0),
        KEY_2 | KEY_KP_2 => Some(1),
        KEY_3 | KEY_KP_3 => Some(2),
        KEY_4 | KEY_KP_4 => Some(3),
        KEY_5 | KEY_KP_5 => Some(4),
        KEY_6 | KEY_KP_6 => Some(5),
        KEY_7 | KEY_KP_7 => Some(6),
        KEY_8 | KEY_KP_8 => Some(7),
        KEY_9 | KEY_KP_9 => Some(8),
        _ => None,
    }
}

/// Port of the Ctrl+Tab rule: Tab switches forward, Shift+Tab backward.
pub fn tab_switch_direction(is_tab_key: bool, shift: bool) -> Option<i32> {
    if is_tab_key {
        Some(if shift { -1 } else { 1 })
    } else {
        None
    }
}

/// Port of the PageUp/PageDown rule: only without Shift, PageDown is
/// forward and PageUp is backward (with Shift the keys fall through to
/// the terminal, exactly like the Python version).
pub fn page_switch_direction(keyval: u32, shift: bool) -> Option<i32> {
    pub const KEY_PAGE_UP: u32 = 0xFF55;
    pub const KEY_PAGE_DOWN: u32 = 0xFF56;
    if shift {
        return None;
    }
    match keyval {
        KEY_PAGE_UP => Some(-1),
        KEY_PAGE_DOWN => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_numbers_map_to_tab_indexes() {
        assert_eq!(alt_number_tab_index(KEY_1), Some(0));
        assert_eq!(alt_number_tab_index(KEY_5), Some(4));
        assert_eq!(alt_number_tab_index(KEY_9), Some(8));
    }

    #[test]
    fn alt_keypad_numbers_map_to_tab_indexes() {
        assert_eq!(alt_number_tab_index(KEY_KP_1), Some(0));
        assert_eq!(alt_number_tab_index(KEY_KP_9), Some(8));
    }

    #[test]
    fn other_keys_do_not_select_tabs() {
        assert_eq!(alt_number_tab_index(0x030), None);
        assert_eq!(alt_number_tab_index(0x0FF), None);
    }

    #[test]
    fn keyvals_match_gdk_constants() {
        assert_eq!(KEY_1, 0x031);
        assert_eq!(KEY_9, 0x039);
        assert_eq!(KEY_KP_1, 0xFFB1);
        assert_eq!(KEY_KP_9, 0xFFB9);
    }

    #[test]
    fn ctrl_tab_directions() {
        assert_eq!(tab_switch_direction(true, false), Some(1));
        assert_eq!(tab_switch_direction(true, true), Some(-1));
        assert_eq!(tab_switch_direction(false, false), None);
    }

    #[test]
    fn page_up_down_directions() {
        assert_eq!(page_switch_direction(0xFF55, false), Some(-1)); // PageUp
        assert_eq!(page_switch_direction(0xFF56, false), Some(1)); // PageDown
        assert_eq!(page_switch_direction(0xFF55, true), None); // with Shift: fall through
        assert_eq!(page_switch_direction(0x062, false), None); // other key
    }
}
