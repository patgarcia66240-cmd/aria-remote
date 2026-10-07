//! Capture de l'écran : une source produit des images brutes RGBA, `screen::encode_jpeg` les compresse pour le canal « frames ».

pub mod screen;

use std::sync::Arc;

use serde::Serialize;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub trait ScreenSource {
    fn capture(&mut self) -> anyhow::Result<Frame>;
}

/// Un écran de l'appareil. Position et taille en pixels du bureau virtuel (l'écran de gauche a x = 0 s'il est principal, un écran à sa droite
/// commence à la largeur du premier...) : c'est ce qui permet de placer la souris sur le bon écran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Screen {
    pub index: usize,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// Les écrans de la machine : leur liste (de gauche à droite) et l'ouverture de l'un d'eux par son rang.
/// `open` est appelée DANS le thread de capture : certaines API d'écran ne sont pas transférables d'un thread à l'autre.
#[derive(Clone)]
pub struct Screens {
    pub list: Arc<dyn Fn() -> Vec<Screen> + Send + Sync>,
    pub open: Arc<dyn Fn(usize) -> anyhow::Result<Box<dyn ScreenSource>> + Send + Sync>,
}

impl Screens {
    /// Rang de l'écran principal (le premier si aucun n'est marqué), 0 s'il n'y a aucun écran.
    pub fn primary(&self) -> usize {
        let screens = (self.list)();
        screens.iter().position(|s| s.primary).unwrap_or(0)
    }
}
