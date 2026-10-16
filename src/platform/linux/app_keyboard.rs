//! Native keyboard translation for the focused isolated WPE surface.
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativeKey {
    pub pressed: bool,
    pub symbol: u32,
    pub code: u32,
    pub modifiers: u32,
}

#[derive(Default)]
pub(super) struct Keyboard {
    held: HashMap<egui::Key, (u32, u32)>,
    modifiers: u32,
}
impl Keyboard {
    pub fn translate(
        &mut self,
        events: &[egui::Event],
        modifiers: egui::Modifiers,
        focused: bool,
    ) -> Vec<NativeKey> {
        let mut out = Vec::new();
        let flags = if focused {
            flags_from_event(modifiers)
        } else {
            0
        };
        if !focused {
            for (_, (symbol, code)) in self.held.drain() {
                out.push(NativeKey {
                    pressed: false,
                    symbol,
                    code,
                    modifiers: self.modifiers,
                });
            }
        }
        for (bit, symbol, code) in [
            (1, 0xffe3, 37),
            (2, 0xffe1, 50),
            (4, 0xffe9, 64),
            (8, 0xffeb, 133),
        ] {
            if (flags ^ self.modifiers) & bit != 0 {
                out.push(NativeKey {
                    pressed: flags & bit != 0,
                    symbol,
                    code,
                    modifiers: flags,
                });
            }
        }
        self.modifiers = flags;
        if !focused {
            return out;
        }
        // egui emits both Key and Text for printable keys. Use the actual text
        // keysym on key-down, not a second synthetic press (including layouts).
        let mut text: VecDeque<char> = events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(s) => Some(s.chars()),
                _ => None,
            })
            .flatten()
            .collect();
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    modifiers,
                    ..
                } => {
                    let Some(mut symbol) = keysym(*key, modifiers.shift) else {
                        continue;
                    };
                    let code = keycode(physical_key.unwrap_or(*key));
                    if *pressed {
                        if symbol < 0xff00
                            && !modifiers.ctrl
                            && !modifiers.alt
                            && !modifiers.mac_cmd
                            && let Some(ch) = text.pop_front()
                        {
                            symbol = unicode(ch);
                        }
                        self.held.insert(*key, (symbol, code));
                    } else if let Some((held_symbol, _)) = self.held.remove(key) {
                        symbol = held_symbol;
                    }
                    out.push(NativeKey {
                        pressed: *pressed,
                        symbol,
                        code,
                        modifiers: flags_from_event(*modifiers),
                    });
                }
                egui::Event::Copy | egui::Event::Cut => {
                    let key = if matches!(event, egui::Event::Copy) {
                        egui::Key::C
                    } else {
                        egui::Key::X
                    };
                    if !events
                        .iter()
                        .any(|e| matches!(e,egui::Event::Key{key:k,..} if *k==key))
                    {
                        for pressed in [true, false] {
                            out.push(NativeKey {
                                pressed,
                                symbol: keysym(key, false).unwrap(),
                                code: keycode(key),
                                modifiers: 1,
                            });
                        }
                    }
                }
                egui::Event::Paste(value) => {
                    for ch in value.chars() {
                        character(&mut out, ch);
                    }
                }
                _ => {}
            }
        }
        // Text without a Key event (for example composed Unicode input).
        for ch in text {
            character(&mut out, ch);
        }
        out
    }
}
fn character(out: &mut Vec<NativeKey>, ch: char) {
    for pressed in [true, false] {
        out.push(NativeKey {
            pressed,
            symbol: unicode(ch),
            code: 0,
            modifiers: 0,
        });
    }
}
fn unicode(ch: char) -> u32 {
    let n = ch as u32;
    if n <= 255 { n } else { 0x01000000 | n }
}
fn flags_from_event(m: egui::Modifiers) -> u32 {
    u32::from(m.ctrl)
        | (u32::from(m.shift) << 1)
        | (u32::from(m.alt) << 2)
        | (u32::from(m.mac_cmd) << 3)
}
fn keysym(key: egui::Key, shift: bool) -> Option<u32> {
    use egui::Key::*;
    Some(match key {
        ArrowLeft => 0xff51,
        ArrowUp => 0xff52,
        ArrowRight => 0xff53,
        ArrowDown => 0xff54,
        Escape => 0xff1b,
        Tab => 0xff09,
        Backspace => 0xff08,
        Enter => 0xff0d,
        Space => 32,
        Insert => 0xff63,
        Delete => 0xffff,
        Home => 0xff50,
        End => 0xff57,
        PageUp => 0xff55,
        PageDown => 0xff56,
        Colon => 58,
        Comma => 44,
        Backslash => 92,
        Slash => 47,
        Pipe => 124,
        Questionmark => 63,
        Exclamationmark => 33,
        OpenBracket => 91,
        CloseBracket => 93,
        OpenCurlyBracket => 123,
        CloseCurlyBracket => 125,
        Backtick => 96,
        Minus => 45,
        Period => 46,
        Plus => 43,
        Equals => 61,
        Semicolon => 59,
        Quote => 39,
        _ => {
            let name = key.name();
            if name.len() == 1 {
                let c = name.as_bytes()[0];
                if c.is_ascii_alphabetic() && !shift {
                    u32::from(c.to_ascii_lowercase())
                } else {
                    u32::from(c)
                }
            } else {
                let f = name
                    .strip_prefix('F')
                    .and_then(|n| n.parse::<u32>().ok())
                    .filter(|n| (1..=35).contains(n))?;
                0xffbd + f
            }
        }
    })
}
// XKB hardware keycodes (evdev + 8), matching WPE's native Wayland input.
fn keycode(key: egui::Key) -> u32 {
    use egui::Key::*;
    match key {
        A => 38,
        B => 56,
        C => 54,
        D => 40,
        E => 26,
        F => 41,
        G => 42,
        H => 43,
        I => 31,
        J => 44,
        K => 45,
        L => 46,
        M => 58,
        N => 57,
        O => 32,
        P => 33,
        Q => 24,
        R => 27,
        S => 39,
        T => 28,
        U => 30,
        V => 55,
        W => 25,
        X => 53,
        Y => 29,
        Z => 52,
        Num1 => 10,
        Num2 => 11,
        Num3 => 12,
        Num4 => 13,
        Num5 => 14,
        Num6 => 15,
        Num7 => 16,
        Num8 => 17,
        Num9 => 18,
        Num0 => 19,
        Escape => 9,
        Backspace => 22,
        Tab => 23,
        Enter => 36,
        Space => 65,
        Minus => 20,
        Equals | Plus => 21,
        OpenBracket | OpenCurlyBracket => 34,
        CloseBracket | CloseCurlyBracket => 35,
        Semicolon | Colon => 47,
        Quote => 48,
        Backtick => 49,
        Backslash | Pipe => 51,
        Comma => 59,
        Period => 60,
        Slash | Questionmark => 61,
        Home => 110,
        ArrowUp => 111,
        PageUp => 112,
        ArrowLeft => 113,
        ArrowRight => 114,
        End => 115,
        ArrowDown => 116,
        PageDown => 117,
        Insert => 118,
        Delete => 119,
        F1 => 67,
        F2 => 68,
        F3 => 69,
        F4 => 70,
        F5 => 71,
        F6 => 72,
        F7 => 73,
        F8 => 74,
        F9 => 75,
        F10 => 76,
        F11 => 95,
        F12 => 96,
        _ => 0,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }
    }
    #[test]
    fn text_is_not_duplicated_and_held_navigation_releases_on_blur() {
        let mut keyboard = Keyboard::default();
        let events = keyboard.translate(
            &[
                key(egui::Key::W, true, egui::Modifiers::NONE),
                egui::Event::Text("w".into()),
            ],
            egui::Modifiers::NONE,
            true,
        );
        assert_eq!(
            events,
            vec![NativeKey {
                pressed: true,
                symbol: 119,
                code: 25,
                modifiers: 0
            }]
        );
        let released = keyboard.translate(&[], egui::Modifiers::NONE, false);
        assert_eq!(
            released,
            vec![NativeKey {
                pressed: false,
                symbol: 119,
                code: 25,
                modifiers: 0
            }]
        );
    }
    #[test]
    fn shortcut_modifiers_function_keys_and_matching_key_up() {
        let mut keyboard = Keyboard::default();
        let mods = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        let down = keyboard.translate(
            &[
                key(egui::Key::K, true, mods),
                key(egui::Key::F5, true, mods),
            ],
            mods,
            true,
        );
        assert!(down.contains(&NativeKey {
            pressed: true,
            symbol: 75,
            code: 45,
            modifiers: 3
        }));
        assert!(down.contains(&NativeKey {
            pressed: true,
            symbol: 0xffc2,
            code: 71,
            modifiers: 3
        }));
        let up = keyboard.translate(
            &[key(egui::Key::K, false, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
            true,
        );
        assert!(up.contains(&NativeKey {
            pressed: false,
            symbol: 75,
            code: 45,
            modifiers: 0
        }));
    }
}
