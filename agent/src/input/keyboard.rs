//! Clavier : `code` du navigateur (touche physique) -> touche nommée ; sinon, le caractère produit (`key` d'un seul caractère).
//! Les raccourcis système réservés (Ctrl+Alt+Suppr, touche Windows seule…) ne sont pas simulables et restent à l'utilisateur de l'appareil.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Named {
    Alt, Backspace, CapsLock, Control, Delete, Down, End, Escape, Home, Insert, Left, Meta, PageDown, PageUp, Return, Right, Shift, Space, Tab, Up,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalKey {
    Named(Named),
    Char(char),
}

pub fn map(code: &str, key: &str) -> Option<LogicalKey> {
    use Named::*;
    let named = match code {
        "AltLeft" | "AltRight" => Alt,
        "Backspace" => Backspace,
        "CapsLock" => CapsLock,
        "ControlLeft" | "ControlRight" => Control,
        "Delete" => Delete,
        "ArrowDown" => Down,
        "End" => End,
        "Escape" => Escape,
        "Home" => Home,
        "Insert" => Insert,
        "ArrowLeft" => Left,
        "MetaLeft" | "MetaRight" => Meta,
        "PageDown" => PageDown,
        "PageUp" => PageUp,
        "Enter" | "NumpadEnter" => Return,
        "ArrowRight" => Right,
        "ShiftLeft" | "ShiftRight" => Shift,
        "Space" => Space,
        "Tab" => Tab,
        "ArrowUp" => Up,
        _ => {
            if let Some(n) = code.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n)) {
                return Some(LogicalKey::Named(F(n)));
            }
            let mut chars = key.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_control() => Some(LogicalKey::Char(c)),
                _ => None,
            };
        }
    };
    Some(LogicalKey::Named(named))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_codes_win_over_the_produced_character() {
        assert_eq!(map("Enter", "Enter"), Some(LogicalKey::Named(Named::Return)));
        assert_eq!(map("ShiftLeft", "Shift"), Some(LogicalKey::Named(Named::Shift)));
        assert_eq!(map("F5", "F5"), Some(LogicalKey::Named(Named::F(5))));
        assert_eq!(map("Space", " "), Some(LogicalKey::Named(Named::Space)));
    }

    #[test]
    fn printable_characters_are_typed_as_they_are() {
        assert_eq!(map("KeyA", "a"), Some(LogicalKey::Char('a')));
        assert_eq!(map("Digit1", "&"), Some(LogicalKey::Char('&')));
        assert_eq!(map("KeyE", "é"), Some(LogicalKey::Char('é')));
    }

    #[test]
    fn unsupported_keys_are_refused_rather_than_guessed() {
        assert_eq!(map("F13", "F13"), None);
        assert_eq!(map("IntlYen", "Unidentified"), None);
        assert_eq!(map("Dead", "Dead"), None);
        assert_eq!(map("KeyA", "\u{7}"), None);
        assert_eq!(map("", ""), None);
    }
}
