//! Capture de l'écran : une source produit des images brutes RGBA, `screen::encode_jpeg` les compresse pour le canal « frames ».

pub mod screen;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub trait ScreenSource {
    fn capture(&mut self) -> anyhow::Result<Frame>;
}
