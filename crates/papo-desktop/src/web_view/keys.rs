//! A key Rune delivered, as the page hears it: an X keysym (WPE's `keyval`) and WPE's modifier
//! bits. Owns only the mapping; which keys reach the page (never the first Esc) is the
//! player's, and turning a character into its keysym is WPE's (`wpe_unicode_to_keyval`).

use rune::Key;
use rune::edit::Mods;

/// A key for the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageKey {
    /// A key with its own keysym (arrows, Return, F-keys…).
    Keysym(u32),
    /// A key that types a character; WPE finds its keysym.
    Char(char),
}

/// A key and the modifiers held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyPress {
    pub(crate) key: PageKey,
    /// WPE's modifier bits (`WPEModifiers`).
    pub(crate) modifiers: u32,
}

const MODIFIER_CONTROL: u32 = 1 << 0;
const MODIFIER_SHIFT: u32 = 1 << 1;
const MODIFIER_ALT: u32 = 1 << 2;
const MODIFIER_META: u32 = 1 << 3;

/// `key` with `mods` for the page, or `None` for what a page can't take (a lone modifier, the
/// Menu key, which opens Papo's menu).
pub(crate) fn key_press(key: &Key, mods: Mods) -> Option<KeyPress> {
    let key = match key {
        Key::Text(text) => PageKey::Char(text.chars().next()?),
        Key::Command(character) => PageKey::Char(*character),
        Key::Enter => PageKey::Keysym(0xff0d),
        Key::Backspace => PageKey::Keysym(0xff08),
        Key::Delete => PageKey::Keysym(0xffff),
        Key::Left => PageKey::Keysym(0xff51),
        Key::Up => PageKey::Keysym(0xff52),
        Key::Right => PageKey::Keysym(0xff53),
        Key::Down => PageKey::Keysym(0xff54),
        Key::Home => PageKey::Keysym(0xff50),
        Key::End => PageKey::Keysym(0xff57),
        Key::PageUp => PageKey::Keysym(0xff55),
        Key::PageDown => PageKey::Keysym(0xff56),
        Key::Insert => PageKey::Keysym(0xff63),
        Key::Escape => PageKey::Keysym(0xff1b),
        Key::Tab if mods.shift => PageKey::Keysym(0xfe20),
        Key::Tab => PageKey::Keysym(0xff09),
        Key::Function(number @ 1..=24) => PageKey::Keysym(0xffbe + u32::from(*number) - 1),
        _ => return None,
    };
    let modifiers = [
        (mods.ctrl, MODIFIER_CONTROL),
        (mods.shift, MODIFIER_SHIFT),
        (mods.alt, MODIFIER_ALT),
        (mods.logo, MODIFIER_META),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .fold(0, |bits, (_, bit)| bits | bit);
    Some(KeyPress { key, modifiers })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(key: &Key) -> Option<PageKey> {
        key_press(key, Mods::default()).map(|press| press.key)
    }

    #[test]
    fn named_keys_have_their_keysyms() {
        assert_eq!(plain(&Key::Enter), Some(PageKey::Keysym(0xff0d)));
        assert_eq!(plain(&Key::Left), Some(PageKey::Keysym(0xff51)));
        assert_eq!(plain(&Key::Escape), Some(PageKey::Keysym(0xff1b)));
        assert_eq!(plain(&Key::Function(1)), Some(PageKey::Keysym(0xffbe)));
        assert_eq!(plain(&Key::Function(11)), Some(PageKey::Keysym(0xffc8)));
    }

    #[test]
    fn typed_keys_are_their_first_character() {
        assert_eq!(plain(&Key::Text("k".into())), Some(PageKey::Char('k')));
        assert_eq!(plain(&Key::Text(" ".into())), Some(PageKey::Char(' ')));
        assert_eq!(plain(&Key::Text(String::new())), None);
    }

    #[test]
    fn shift_tab_is_its_own_keysym_and_modifiers_map_to_wpe_bits() {
        let mods = Mods {
            shift: true,
            ctrl: true,
            ..Mods::default()
        };
        let press = key_press(&Key::Tab, mods).unwrap();
        assert_eq!(press.key, PageKey::Keysym(0xfe20));
        assert_eq!(press.modifiers, MODIFIER_SHIFT | MODIFIER_CONTROL);
        let command = key_press(
            &Key::Command('c'),
            Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            },
        );
        assert_eq!(command.unwrap().key, PageKey::Char('c'));
    }

    #[test]
    fn the_menu_key_stays_papos() {
        assert_eq!(plain(&Key::ContextMenu), None);
    }
}
