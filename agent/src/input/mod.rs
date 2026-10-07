//! Commandes reçues du contrôleur (canal « control ») et leur application. La permission est vérifiée ICI, côté agent, pour chaque message :
//! l'agent ne fait jamais confiance au serveur ni au contrôleur pour décider de ce qu'il a le droit d'exécuter (docs/REMOTE_ARCHITECTURE.md, §15).

pub mod keyboard;
pub mod mouse;

use std::collections::HashSet;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    ViewScreen,
    ControlMouse,
    ControlKeyboard,
}

impl Permission {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "view_screen" => Some(Self::ViewScreen),
            "control_mouse" => Some(Self::ControlMouse),
            "control_keyboard" => Some(Self::ControlKeyboard),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ViewScreen => "view_screen",
            Self::ControlMouse => "control_mouse",
            Self::ControlKeyboard => "control_keyboard",
        }
    }
}

/// Droit de regarder toujours accordé ; souris et clavier seulement si l'utilisateur de l'appareil les a autorisés (`--allow`).
pub fn parse_allow(list: &str) -> HashSet<Permission> {
    let mut allowed = HashSet::from([Permission::ViewScreen]);
    for item in list.split(',').map(|s| s.trim().to_lowercase()) {
        match item.as_str() {
            "mouse" | "souris" => { allowed.insert(Permission::ControlMouse); }
            "keyboard" | "clavier" => { allowed.insert(Permission::ControlKeyboard); }
            _ => {}
        }
    }
    allowed
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Control {
    Move { x: f64, y: f64 },
    Down { x: f64, y: f64, button: u8 },
    Up { x: f64, y: f64, button: u8 },
    Wheel { dx: f64, dy: f64 },
    Key { down: bool, code: String, key: String },
}

impl Control {
    pub fn required(&self) -> Permission {
        match self {
            Self::Key { .. } => Permission::ControlKeyboard,
            _ => Permission::ControlMouse,
        }
    }
}

/// Ce que l'agent sait faire sur l'appareil. Implémenté pour Windows (enigo) ; ailleurs, une version qui journalise.
pub trait InputSink: Send {
    fn display_size(&self) -> (u32, u32);
    /// Position actuelle du pointeur en pixels (le contrôleur la dessine : la capture d'écran ne contient pas le curseur).
    fn cursor(&self) -> Option<(i32, i32)> { None }
    fn move_to(&mut self, x: i32, y: i32);
    fn button(&mut self, button: mouse::Button, down: bool);
    fn wheel(&mut self, dx: i32, dy: i32);
    fn key(&mut self, key: keyboard::LogicalKey, down: bool);
}

/// Applique une commande si la session l'a le droit ; sinon l'ignore et renvoie la raison (jamais d'exécution silencieuse).
pub fn apply(message: &Control, allowed: &HashSet<Permission>, sink: &mut dyn InputSink) -> Result<(), &'static str> {
    if !allowed.contains(&message.required()) {
        return Err("permission refusée");
    }
    let (width, height) = sink.display_size();
    match message {
        Control::Move { x, y } => {
            let (px, py) = mouse::to_pixels(*x, *y, width, height);
            sink.move_to(px, py);
        }
        Control::Down { x, y, button } | Control::Up { x, y, button } => {
            let down = matches!(message, Control::Down { .. });
            let (px, py) = mouse::to_pixels(*x, *y, width, height);
            sink.move_to(px, py);
            sink.button(mouse::Button::from_dom(*button).ok_or("bouton inconnu")?, down);
        }
        Control::Wheel { dx, dy } => sink.wheel(mouse::wheel_steps(*dx), mouse::wheel_steps(*dy)),
        Control::Key { down, code, key } => sink.key(keyboard::map(code, key).ok_or("touche non prise en charge")?, *down),
    }
    Ok(())
}

/// Saisie qui ne touche à rien : développement et tests hors Windows.
#[cfg_attr(windows, allow(dead_code))]      // sous Windows, la saisie réelle (enigo) la remplace ; elle sert ailleurs et dans les tests
pub struct LogInput {
    pub log: Vec<String>,
    echo: bool,
    pointer: Option<(i32, i32)>,
}

#[cfg_attr(windows, allow(dead_code))]
impl LogInput {
    #[cfg(test)]
    pub fn new() -> Self {
        Self { log: Vec::new(), echo: false, pointer: None }
    }

    /// Affiche chaque action sur la sortie d'erreur : on voit ce que le contrôleur ferait sur un vrai appareil.
    pub fn echoing() -> Self {
        Self { log: Vec::new(), echo: true, pointer: None }
    }

    fn record(&mut self, line: String) {
        if self.echo {
            eprintln!("[saisie simulée] {line}");
        }
        self.log.push(line);
    }
}

#[cfg_attr(windows, allow(dead_code))]
impl InputSink for LogInput {
    fn display_size(&self) -> (u32, u32) { (1280, 720) }
    fn cursor(&self) -> Option<(i32, i32)> { self.pointer }
    fn move_to(&mut self, x: i32, y: i32) { self.pointer = Some((x, y)); self.record(format!("move {x},{y}")); }
    fn button(&mut self, button: mouse::Button, down: bool) { self.record(format!("button {button:?} {down}")); }
    fn wheel(&mut self, dx: i32, dy: i32) { self.record(format!("wheel {dx},{dy}")); }
    fn key(&mut self, key: keyboard::LogicalKey, down: bool) { self.record(format!("key {key:?} {down}")); }
}

#[cfg(windows)]
pub mod native;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Control { serde_json::from_str(text).unwrap() }

    #[test]
    fn every_command_needs_its_own_permission() {
        let mut sink = LogInput::new();
        let view_only = HashSet::from([Permission::ViewScreen]);
        let mouse = HashSet::from([Permission::ViewScreen, Permission::ControlMouse]);
        assert_eq!(apply(&parse(r#"{"t":"move","x":0.5,"y":0.5}"#), &view_only, &mut sink), Err("permission refusée"));
        assert_eq!(apply(&parse(r#"{"t":"key","down":true,"code":"KeyA","key":"a"}"#), &mouse, &mut sink), Err("permission refusée"));
        assert!(sink.log.is_empty(), "rien ne doit être exécuté sans permission");
        assert_eq!(apply(&parse(r#"{"t":"move","x":0.5,"y":0.5}"#), &mouse, &mut sink), Ok(()));
        assert_eq!(sink.log, vec!["move 640,360"]);
    }

    #[test]
    fn clicks_move_first_and_unknown_buttons_or_keys_are_refused() {
        let mut sink = LogInput::new();
        let all = HashSet::from([Permission::ViewScreen, Permission::ControlMouse, Permission::ControlKeyboard]);
        assert_eq!(apply(&parse(r#"{"t":"down","x":0,"y":1,"button":2}"#), &all, &mut sink), Ok(()));
        assert_eq!(sink.log, vec!["move 0,719", "button Right true"]);
        assert_eq!(apply(&parse(r#"{"t":"down","x":0,"y":0,"button":9}"#), &all, &mut sink), Err("bouton inconnu"));
        assert_eq!(apply(&parse(r#"{"t":"key","down":true,"code":"IntlYen","key":"Unidentified"}"#), &all, &mut sink), Err("touche non prise en charge"));
        assert_eq!(apply(&parse(r#"{"t":"key","down":false,"code":"Enter","key":"Enter"}"#), &all, &mut sink), Ok(()));
    }

    #[test]
    fn the_cursor_position_follows_the_last_move() {
        let mut sink = LogInput::new();
        let mouse = HashSet::from([Permission::ViewScreen, Permission::ControlMouse]);
        assert_eq!(sink.cursor(), None);
        apply(&parse(r#"{"t":"move","x":0.25,"y":0.5}"#), &mouse, &mut sink).unwrap();
        assert_eq!(sink.cursor(), Some((320, 360)));
    }

    #[test]
    fn malformed_commands_do_not_parse() {
        assert!(serde_json::from_str::<Control>(r#"{"t":"shell","cmd":"del *"}"#).is_err());
        assert!(serde_json::from_str::<Control>(r#"{"t":"move"}"#).is_err());
        assert!(serde_json::from_str::<Control>("pas du json").is_err());
    }

    #[test]
    fn allow_always_includes_viewing_and_ignores_unknown_words() {
        assert_eq!(parse_allow(""), HashSet::from([Permission::ViewScreen]));
        let both = parse_allow("Mouse, clavier, terminal");
        assert_eq!(both.len(), 3);
        assert!(!both.iter().any(|p| p.as_str() == "terminal"));
    }
}
