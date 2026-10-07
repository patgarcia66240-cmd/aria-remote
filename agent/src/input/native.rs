//! Saisie réelle sous Windows (enigo). Compilé uniquement sur Windows.

use enigo::{Axis, Button as EButton, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

use super::keyboard::{LogicalKey, Named};
use super::mouse::Button;
use super::InputSink;

pub struct NativeInput {
    enigo: Enigo,
}

impl NativeInput {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self { enigo: Enigo::new(&Settings::default())? })
    }
}

fn named(key: Named) -> Key {
    match key {
        Named::Alt => Key::Alt,
        Named::Backspace => Key::Backspace,
        Named::CapsLock => Key::CapsLock,
        Named::Control => Key::Control,
        Named::Delete => Key::Delete,
        Named::Down => Key::DownArrow,
        Named::End => Key::End,
        Named::Escape => Key::Escape,
        Named::Home => Key::Home,
        Named::Insert => Key::Insert,
        Named::Left => Key::LeftArrow,
        Named::Meta => Key::Meta,
        Named::PageDown => Key::PageDown,
        Named::PageUp => Key::PageUp,
        Named::Return => Key::Return,
        Named::Right => Key::RightArrow,
        Named::Shift => Key::Shift,
        Named::Space => Key::Space,
        Named::Tab => Key::Tab,
        Named::Up => Key::UpArrow,
        Named::F(n) => match n {
            1 => Key::F1, 2 => Key::F2, 3 => Key::F3, 4 => Key::F4, 5 => Key::F5, 6 => Key::F6,
            7 => Key::F7, 8 => Key::F8, 9 => Key::F9, 10 => Key::F10, 11 => Key::F11, _ => Key::F12,
        },
    }
}

impl InputSink for NativeInput {
    fn display_size(&self) -> (u32, u32) {
        self.enigo.main_display().map(|(w, h)| (w.max(1) as u32, h.max(1) as u32)).unwrap_or((1920, 1080))
    }

    fn cursor(&self) -> Option<(i32, i32)> {
        self.enigo.location().ok()
    }

    fn move_to(&mut self, x: i32, y: i32) {
        let _ = self.enigo.move_mouse(x, y, Coordinate::Abs);
    }

    fn button(&mut self, button: Button, down: bool) {
        let button = match button {
            Button::Left => EButton::Left,
            Button::Middle => EButton::Middle,
            Button::Right => EButton::Right,
            Button::Back => EButton::Back,
            Button::Forward => EButton::Forward,
        };
        let _ = self.enigo.button(button, if down { Direction::Press } else { Direction::Release });
    }

    fn wheel(&mut self, dx: i32, dy: i32) {
        if dy != 0 { let _ = self.enigo.scroll(dy, Axis::Vertical); }
        if dx != 0 { let _ = self.enigo.scroll(dx, Axis::Horizontal); }
    }

    fn key(&mut self, key: LogicalKey, down: bool) {
        let key = match key {
            LogicalKey::Named(named_key) => named(named_key),
            LogicalKey::Char(c) => Key::Unicode(c),
        };
        let _ = self.enigo.key(key, if down { Direction::Press } else { Direction::Release });
    }
}
