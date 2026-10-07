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

/// Zone de l'écran contrôlé, en pixels du bureau virtuel : les coordonnées 0..1 du contrôleur y sont rapportées (écran choisi, pas forcément le principal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Ce que l'agent sait faire sur l'appareil. Implémenté pour Windows (enigo) ; ailleurs, une version qui journalise.
pub trait InputSink: Send {
    fn display_size(&self) -> (u32, u32);
    /// Zone contrôlée : celle choisie avec `set_area`, sinon l'écran principal à l'origine.
    fn area(&self) -> Area {
        let (width, height) = self.display_size();
        Area { x: 0, y: 0, width, height }
    }
    /// Choisit l'écran sur lequel s'appliquent les coordonnées du contrôleur.
    fn set_area(&mut self, _area: Area) {}
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
    let area = sink.area();
    let to_screen = |x: f64, y: f64| {
        let (px, py) = mouse::to_pixels(x, y, area.width, area.height);
        (area.x.saturating_add(px), area.y.saturating_add(py))
    };
    match message {
        Control::Move { x, y } => {
            let (px, py) = to_screen(*x, *y);
            sink.move_to(px, py);
        }
        Control::Down { x, y, button } | Control::Up { x, y, button } => {
            let down = matches!(message, Control::Down { .. });
            let (px, py) = to_screen(*x, *y);
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
    area: Option<Area>,
}

#[cfg_attr(windows, allow(dead_code))]
impl LogInput {
    #[cfg(test)]
    pub fn new() -> Self {
        Self { log: Vec::new(), echo: false, pointer: None, area: None }
    }

    /// Affiche chaque action sur la sortie d'erreur : on voit ce que le contrôleur ferait sur un vrai appareil.
    pub fn echoing() -> Self {
        Self { log: Vec::new(), echo: true, pointer: None, area: None }
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
    fn area(&self) -> Area { self.area.unwrap_or(Area { x: 0, y: 0, width: 1280, height: 720 }) }
    fn set_area(&mut self, area: Area) { self.area = Some(area); }
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
    fn coordinates_are_relative_to_the_chosen_screen() {
        let mut sink = LogInput::new();
        let mouse = HashSet::from([Permission::ViewScreen, Permission::ControlMouse]);
        sink.set_area(Area { x: 1280, y: 0, width: 800, height: 600 });
        apply(&parse(r#"{"t":"move","x":0.5,"y":0.5}"#), &mouse, &mut sink).unwrap();
        apply(&parse(r#"{"t":"down","x":0,"y":0,"button":0}"#), &mouse, &mut sink).unwrap();
        apply(&parse(r#"{"t":"move","x":1,"y":1}"#), &mouse, &mut sink).unwrap();
        assert_eq!(sink.log, vec!["move 1680,300", "move 1280,0", "button Left true", "move 2079,599"]);
        // un écran placé à gauche de l'écran principal a des coordonnées négatives
        sink.set_area(Area { x: -1920, y: -200, width: 1920, height: 1080 });
        apply(&parse(r#"{"t":"move","x":0,"y":0}"#), &mouse, &mut sink).unwrap();
        assert_eq!(sink.log.last().unwrap(), "move -1920,-200");
        assert_eq!(sink.area(), Area { x: -1920, y: -200, width: 1920, height: 1080 });
    }

    #[test]
    fn without_a_chosen_screen_the_primary_display_is_used() {
        let sink = LogInput::new();
        assert_eq!(sink.area(), Area { x: 0, y: 0, width: 1280, height: 720 });
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
