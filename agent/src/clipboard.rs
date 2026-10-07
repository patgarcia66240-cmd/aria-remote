//! Presse-papiers partagé (texte seulement) entre l'appareil et le contrôleur.
//!
//! Il n'est actif que si la session a la permission « clavier » : coller du texte à distance revient à taper au clavier. Le texte est limité
//! (`MAX_BYTES`), et ce qui vient d'être reçu n'est jamais renvoyé à l'expéditeur (sinon les deux PC se renverraient le même texte sans fin).

/// Taille maximale d'un texte partagé (256 Ko) : assez pour du code ou un long document, pas pour saturer le canal.
pub const MAX_BYTES: usize = 256 * 1024;

pub trait Clipboard: Send {
    fn get_text(&mut self) -> Option<String>;
    fn set_text(&mut self, text: &str) -> bool;
}

/// Le presse-papiers du système. Une instance `arboard` est créée à chaque appel : c'est léger et évite de la garder entre deux fils.
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }

    fn set_text(&mut self, text: &str) -> bool {
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.to_string())).is_ok()
    }
}

/// Mémoire de ce qui a déjà été échangé : décide quoi envoyer et quoi appliquer.
pub struct Sync {
    last: Option<String>,
}

impl Sync {
    /// `initial` : le contenu actuel du presse-papiers. Il n'est PAS envoyé à l'ouverture de la session (on ne partage que ce qui change ensuite).
    pub fn new(initial: Option<String>) -> Self {
        Self { last: initial }
    }

    /// Contenu actuel du presse-papiers local. Renvoie le texte à envoyer s'il a changé depuis le dernier échange et qu'il est partageable.
    pub fn changed(&mut self, current: Option<String>) -> Option<String> {
        let text = current?;
        if self.last.as_deref() == Some(text.as_str()) {
            return None;
        }
        self.last = Some(text.clone());
        (!text.is_empty() && text.len() <= MAX_BYTES).then_some(text)
    }

    /// Texte reçu de l'autre PC : l'applique et le retient (il ne repartira pas). Refuse l'excès sans toucher au presse-papiers.
    pub fn apply(&mut self, text: &str, clipboard: &mut dyn Clipboard) -> Result<(), &'static str> {
        if text.len() > MAX_BYTES {
            return Err("texte trop gros pour le presse-papiers");
        }
        if !clipboard.set_text(text) {
            return Err("presse-papiers indisponible");
        }
        self.last = Some(text.to_string());
        Ok(())
    }
}

#[cfg(test)]
pub struct MemoryClipboard(pub Option<String>);

#[cfg(test)]
impl Clipboard for MemoryClipboard {
    fn get_text(&mut self) -> Option<String> { self.0.clone() }
    fn set_text(&mut self, text: &str) -> bool { self.0 = Some(text.to_string()); true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_existing_content_is_not_shared_only_what_changes_afterwards() {
        let mut sync = Sync::new(Some("déjà là".into()));
        assert_eq!(sync.changed(Some("déjà là".into())), None);
        assert_eq!(sync.changed(Some("nouveau".into())), Some("nouveau".into()));
        assert_eq!(sync.changed(Some("nouveau".into())), None, "le même texte n'est envoyé qu'une fois");
        assert_eq!(sync.changed(None), None, "presse-papiers illisible ou vide de texte");
    }

    #[test]
    fn what_was_just_received_is_never_sent_back() {
        let mut sync = Sync::new(None);
        let mut clipboard = MemoryClipboard(None);
        sync.apply("venu du contrôleur", &mut clipboard).unwrap();
        assert_eq!(clipboard.0.as_deref(), Some("venu du contrôleur"));
        assert_eq!(sync.changed(clipboard.get_text()), None, "pas d'écho vers l'expéditeur");
        assert_eq!(sync.changed(Some("copié ici ensuite".into())), Some("copié ici ensuite".into()));
    }

    #[test]
    fn oversized_or_empty_text_is_not_shared() {
        let mut sync = Sync::new(None);
        let mut clipboard = MemoryClipboard(Some("avant".into()));
        let big = "x".repeat(MAX_BYTES + 1);
        assert_eq!(sync.apply(&big, &mut clipboard), Err("texte trop gros pour le presse-papiers"));
        assert_eq!(clipboard.0.as_deref(), Some("avant"), "rien n'est écrit quand le texte est refusé");
        assert_eq!(sync.changed(Some(big)), None);
        assert_eq!(sync.changed(Some(String::new())), None);
        assert!(sync.apply(&"y".repeat(MAX_BYTES), &mut clipboard).is_ok(), "la limite est incluse");
    }

    #[test]
    fn a_failing_clipboard_is_reported() {
        struct Broken;
        impl Clipboard for Broken {
            fn get_text(&mut self) -> Option<String> { None }
            fn set_text(&mut self, _: &str) -> bool { false }
        }
        let mut sync = Sync::new(None);
        assert_eq!(sync.apply("x", &mut Broken), Err("presse-papiers indisponible"));
        assert_eq!(sync.changed(Some("x".into())), Some("x".into()), "rien n'a été retenu : le texte reste à envoyer s'il apparaît localement");
    }
}
