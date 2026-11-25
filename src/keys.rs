//! Key handling by hardware keycode, so the same physical keys work in any
//! keyboard layout (us, ru, ua, de, ...). Keycodes are evdev codes + 8.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Down,
    Up,
    PageDown,
    PageUp,
    First,
    Last,
    ToggleMark,
    Copy,
    Paste,
    StartSearch,
    EndSearch,
    Close,
}

pub mod code {
    pub const ESC: u32 = 9;
    pub const Q: u32 = 24;
    pub const P: u32 = 33;
    pub const RETURN: u32 = 36;
    pub const G: u32 = 42;
    pub const J: u32 = 44;
    pub const K: u32 = 45;
    pub const SLASH: u32 = 61;
    pub const SPACE: u32 = 65;
    pub const KP_ENTER: u32 = 104;
    pub const HOME: u32 = 110;
    pub const UP: u32 = 111;
    pub const PAGE_UP: u32 = 112;
    pub const END: u32 = 115;
    pub const DOWN: u32 = 116;
    pub const PAGE_DOWN: u32 = 117;
}

/// What a key press does. `None` means: in normal mode ignore it, in search
/// mode let the search field have it.
pub fn action(mode: Mode, keycode: u32, shift: bool) -> Option<Action> {
    use Action::*;
    use code::*;

    // Shared by both modes.
    let common = match keycode {
        DOWN => Some(Down),
        UP => Some(Up),
        PAGE_DOWN => Some(PageDown),
        PAGE_UP => Some(PageUp),
        SPACE => Some(ToggleMark),
        RETURN | KP_ENTER => Some(Copy),
        _ => None,
    };
    if common.is_some() {
        return common;
    }

    match mode {
        Mode::Normal => match keycode {
            J => Some(Down),
            K => Some(Up),
            G if shift => Some(Last),
            G | HOME => Some(First),
            END => Some(Last),
            P => Some(Paste),
            SLASH => Some(StartSearch),
            ESC | Q => Some(Close),
            _ => None,
        },
        Mode::Search => match keycode {
            ESC => Some(EndSearch),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Action::*, Mode::*, code::*, *};

    #[test]
    fn normal_mode_vim_keys() {
        assert_eq!(action(Normal, J, false), Some(Down));
        assert_eq!(action(Normal, K, false), Some(Up));
        assert_eq!(action(Normal, G, false), Some(First));
        assert_eq!(action(Normal, G, true), Some(Last));
        assert_eq!(action(Normal, P, false), Some(Paste));
        assert_eq!(action(Normal, SLASH, false), Some(StartSearch));
        assert_eq!(action(Normal, Q, false), Some(Close));
        assert_eq!(action(Normal, ESC, false), Some(Close));
    }

    #[test]
    fn shared_keys() {
        for mode in [Normal, Search] {
            assert_eq!(action(mode, SPACE, false), Some(ToggleMark));
            assert_eq!(action(mode, RETURN, false), Some(Copy));
            assert_eq!(action(mode, KP_ENTER, false), Some(Copy));
            assert_eq!(action(mode, DOWN, false), Some(Down));
            assert_eq!(action(mode, UP, false), Some(Up));
        }
    }

    #[test]
    fn search_mode_letters_go_to_the_field() {
        for key in [J, K, G, P, Q, SLASH] {
            assert_eq!(action(Search, key, false), None);
        }
        assert_eq!(action(Search, ESC, false), Some(EndSearch));
    }

    #[test]
    fn unknown_keys_do_nothing() {
        assert_eq!(action(Normal, 38 /* a */, false), None);
    }
}
