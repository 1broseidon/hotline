//! The person's hands on a teammate's browser: pointer and keyboard events
//! put into its X display with XTEST, as a real pointer and keyboard would
//! make them, so a page hovers, drags, scrolls and types as it does for
//! anyone. Ported from the computer's own hands (hotline-computer,
//! `src/xtest.rs`, George's, MIT/Apache), less what only an agent there
//! does (typing whole strings, intents).
//!
//! A browser key event names a key ("a", "Enter", "Shift"); the server
//! wants a keycode. Keysyms bridge the two: the name becomes a keysym, the
//! keyboard mapping says which keycode carries it, and a keysym no keycode
//! carries is given a spare keycode for the life of the display, the way
//! xdotool does, so any character types without an input method. The
//! person's modifiers arrive as their own key events, so pressing the
//! keycode that carries `a` while Shift is down is how they type `A`. A
//! character that only a shifted key carries, sent with no Shift down (a
//! phone's keyboard sends `A` alone), is typed with Shift held around it.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConfigureWindowAux, ConnectionExt, InputFocus,
    KEY_PRESS_EVENT, KEY_RELEASE_EVENT, Keycode, Keysym, MOTION_NOTIFY_EVENT, StackMode, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

const SHIFT: Keysym = 0xffe1;
const CONTROL: Keysym = 0xffe3;
const ALT: Keysym = 0xffe9;
const SUPER: Keysym = 0xffeb;
/// Between a press and its release in a chord, so a client that reads its
/// queue in order still sees two events.
const KEY_GAP: Duration = Duration::from_millis(2);
/// After a keycode is remapped, before it is pressed: clients reload their
/// keymap on MappingNotify, and a press that arrives first types nothing.
const REMAP_SETTLE: Duration = Duration::from_millis(100);

pub struct Hands {
    connection: RustConnection,
    root: Window,
    min_keycode: Keycode,
    per_keycode: usize,
    keysyms: Vec<Keysym>,
    spare: Vec<Keycode>,
    remapped: HashMap<Keysym, Keycode>,
    /// A keycode was remapped since the last key press.
    unsettled: bool,
    /// Keys and buttons put down and not yet let go, so a viewer that
    /// vanishes with one down does not leave it down.
    held: HashSet<Keycode>,
    buttons: HashSet<u8>,
}

impl Hands {
    pub fn new(display: &str) -> Result<Self, String> {
        let (connection, screen_number) =
            x11rb::connect(Some(display)).map_err(|error| format!("x11 connect: {error}"))?;
        let root = connection.setup().roots[screen_number].root;
        let min_keycode = connection.setup().min_keycode;
        let max_keycode = connection.setup().max_keycode;
        let mapping = connection
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| format!("keyboard mapping: {error}"))?;
        let per_keycode = usize::from(mapping.keysyms_per_keycode);
        let spare = (min_keycode..=max_keycode)
            .filter(|keycode| {
                let start = usize::from(keycode - min_keycode) * per_keycode;
                mapping.keysyms[start..start + per_keycode]
                    .iter()
                    .all(|keysym| *keysym == 0)
            })
            .collect();
        Ok(Self {
            connection,
            root,
            min_keycode,
            per_keycode,
            keysyms: mapping.keysyms,
            spare,
            remapped: HashMap::new(),
            unsettled: false,
            held: HashSet::new(),
            buttons: HashSet::new(),
        })
    }

    /// Puts the page's `window` in front, under the pointer, and gives it the
    /// keyboard, before a gesture meant for it: every page is a window at
    /// the display's origin, and the one display's pointer and keys are
    /// shared. Asked again before every gesture, since a window shown since
    /// may be on top; for the window already there it changes nothing.
    pub fn front(&mut self, window: Window) -> Result<(), String> {
        self.connection
            .configure_window(
                window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )
            .map_err(|error| error.to_string())?;
        self.connection
            .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn move_to(&mut self, x: i16, y: i16) -> Result<(), String> {
        self.fake(MOTION_NOTIFY_EVENT, 0, x, y)
    }

    pub fn button(&mut self, button: u8, down: bool) -> Result<(), String> {
        let kind = if down {
            self.buttons.insert(button);
            BUTTON_PRESS_EVENT
        } else {
            self.buttons.remove(&button);
            BUTTON_RELEASE_EVENT
        };
        self.fake(kind, button, 0, 0)
    }

    /// One wheel notch each way asked: X's buttons 4 to 7, pressed and let go.
    pub fn wheel(&mut self, dx: i8, dy: i8) -> Result<(), String> {
        for (turned, button) in [(dy < 0, 4), (dy > 0, 5), (dx < 0, 6), (dx > 0, 7)] {
            if turned {
                self.fake(BUTTON_PRESS_EVENT, button, 0, 0)?;
                self.fake(BUTTON_RELEASE_EVENT, button, 0, 0)?;
            }
        }
        Ok(())
    }

    pub fn key(&mut self, name: &str, down: bool) -> Result<(), String> {
        let Some(keysym) = keysym_for(name) else {
            return Ok(());
        };
        let (keycode, shifted) = self.key_for(keysym)?;
        if !down {
            self.held.remove(&keycode);
            return self.fake(KEY_RELEASE_EVENT, keycode, 0, 0);
        }
        self.held.insert(keycode);
        let shift = self.keycode_for(SHIFT)?;
        if !shifted || self.held.contains(&shift) {
            return self.fake(KEY_PRESS_EVENT, keycode, 0, 0);
        }
        self.fake(KEY_PRESS_EVENT, shift, 0, 0)?;
        self.fake(KEY_PRESS_EVENT, keycode, 0, 0)?;
        self.fake(KEY_RELEASE_EVENT, shift, 0, 0)
    }

    /// Lets go of every key and pointer button still down: what a person's
    /// hands do when they leave, and what a viewer that went away cannot
    /// send for itself.
    pub fn release_all(&mut self) -> Result<(), String> {
        let buttons: Vec<u8> = self.buttons.drain().collect();
        for button in buttons {
            self.fake(BUTTON_RELEASE_EVENT, button, 0, 0)?;
        }
        let held: Vec<Keycode> = self.held.drain().collect();
        for keycode in held {
            self.fake(KEY_RELEASE_EVENT, keycode, 0, 0)?;
        }
        Ok(())
    }

    /// A chord such as `ctrl+v`: modifiers down, the key pressed and
    /// released, modifiers up in reverse.
    pub fn combo(&mut self, combo: &str) -> Result<(), String> {
        let mut parts: Vec<&str> = combo.split('+').map(str::trim).collect();
        let key = parts
            .pop()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| "combo is required".to_owned())?;
        let mut modifiers = Vec::new();
        for part in parts {
            let keysym = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => CONTROL,
                "shift" => SHIFT,
                "alt" => ALT,
                "super" | "meta" => SUPER,
                other => return Err(format!("unknown modifier {other:?}")),
            };
            modifiers.push(self.keycode_for(keysym)?);
        }
        // Shortcut notation such as Ctrl+V names the V key.
        let keysym =
            if !modifiers.is_empty() && key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
                Some(u32::from(key.as_bytes()[0].to_ascii_lowercase()))
            } else {
                keysym_for(key)
            }
            .ok_or_else(|| format!("unknown key {key:?}"))?;
        let keycode = self.keycode_for(keysym)?;
        for modifier in &modifiers {
            self.fake(KEY_PRESS_EVENT, *modifier, 0, 0)?;
        }
        self.fake(KEY_PRESS_EVENT, keycode, 0, 0)?;
        std::thread::sleep(KEY_GAP);
        self.fake(KEY_RELEASE_EVENT, keycode, 0, 0)?;
        for modifier in modifiers.iter().rev() {
            self.fake(KEY_RELEASE_EVENT, *modifier, 0, 0)?;
        }
        Ok(())
    }

    fn fake(&mut self, kind: u8, detail: u8, x: i16, y: i16) -> Result<(), String> {
        // MappingNotify reaches every client; unread, it would pile up in the
        // socket for the life of the display.
        while let Ok(Some(_)) = self.connection.poll_for_event() {}
        if kind == KEY_PRESS_EVENT && std::mem::take(&mut self.unsettled) {
            std::thread::sleep(REMAP_SETTLE);
        }
        self.connection
            .xtest_fake_input(kind, detail, x11rb::CURRENT_TIME, self.root, x, y, 0)
            .map_err(|error| error.to_string())?;
        self.connection.flush().map_err(|error| error.to_string())
    }

    /// Any keycode that carries the keysym.
    fn keycode_for(&mut self, keysym: Keysym) -> Result<Keycode, String> {
        self.key_for(keysym).map(|(keycode, _)| keycode)
    }

    /// The keycode that carries the keysym, and whether it sits in the
    /// shifted column: the first column's, the second's, any column's, or a
    /// spare one mapped to it now.
    fn key_for(&mut self, keysym: Keysym) -> Result<(Keycode, bool), String> {
        if let Some(keycode) = self.remapped.get(&keysym) {
            return Ok((*keycode, false));
        }
        for column in [0, 1] {
            let found = self
                .keysyms
                .chunks(self.per_keycode)
                .position(|columns| columns.get(column) == Some(&keysym))
                .map(|index| self.min_keycode + index as u8);
            if let Some(keycode) = found {
                return Ok((keycode, column == 1));
            }
        }
        let found = self
            .keysyms
            .chunks(self.per_keycode)
            .position(|columns| columns.contains(&keysym))
            .map(|index| self.min_keycode + index as u8);
        if let Some(keycode) = found {
            return Ok((keycode, false));
        }
        let keycode = self
            .spare
            .pop()
            .ok_or_else(|| format!("no spare keycode for keysym {keysym:#x}"))?;
        let columns = vec![keysym; self.per_keycode];
        self.connection
            .change_keyboard_mapping(1, keycode, self.per_keycode as u8, &columns)
            .map_err(|error| error.to_string())?
            .check()
            .map_err(|error| format!("remap keycode: {error}"))?;
        let start = usize::from(keycode - self.min_keycode) * self.per_keycode;
        self.keysyms[start..start + self.per_keycode].copy_from_slice(&columns);
        self.remapped.insert(keysym, keycode);
        self.unsettled = true;
        Ok((keycode, false))
    }
}

/// The X keysym for a browser `KeyboardEvent.key`. A single character is
/// its own keysym below U+0100 and a Unicode keysym above; a named key is
/// looked up. Dead and unidentified keys are nothing to press.
pub fn keysym_for(name: &str) -> Option<Keysym> {
    let mut chars = name.chars();
    if let (Some(character), None) = (chars.next(), chars.next()) {
        let code = character as u32;
        if code < 0x20 {
            return None;
        }
        return Some(if code < 0x100 {
            code
        } else {
            0x0100_0000 | code
        });
    }
    if let Some(number) = name.strip_prefix('F')
        && let Ok(number) = number.parse::<u32>()
        && (1..=12).contains(&number)
    {
        return Some(0xffbe + number - 1);
    }
    Some(match name {
        "Enter" => 0xff0d,
        "Tab" => 0xff09,
        "Backspace" => 0xff08,
        "Escape" => 0xff1b,
        "Delete" => 0xffff,
        "Insert" => 0xff63,
        "Home" => 0xff50,
        "End" => 0xff57,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "Shift" => SHIFT,
        "Control" => CONTROL,
        "Alt" => ALT,
        "AltGraph" => 0xfe03,
        "Meta" => SUPER,
        "CapsLock" => 0xffe5,
        "NumLock" => 0xff7f,
        "ScrollLock" => 0xff14,
        "ContextMenu" => 0xff67,
        "PrintScreen" => 0xff61,
        "Pause" => 0xff13,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_are_their_own_keysyms() {
        assert_eq!(keysym_for("a"), Some(0x61));
        assert_eq!(keysym_for("A"), Some(0x41));
        assert_eq!(keysym_for(" "), Some(0x20));
        assert_eq!(keysym_for("é"), Some(0xe9));
        assert_eq!(keysym_for("€"), Some(0x0100_20ac));
        assert_eq!(keysym_for("\u{7}"), None, "a control character");
    }

    #[test]
    fn named_keys_are_looked_up() {
        assert_eq!(keysym_for("Enter"), Some(0xff0d));
        assert_eq!(keysym_for("F5"), Some(0xffc2));
        assert_eq!(keysym_for("F13"), None);
        assert_eq!(keysym_for("Dead"), None);
        assert_eq!(keysym_for("Shift"), Some(SHIFT));
    }
}
