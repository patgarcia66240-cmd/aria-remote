//! Sources d'écran et compression JPEG. Windows : écran principal via xcap. Ailleurs (développement, tests) : image synthétique animée.

use std::io::Cursor;

use anyhow::{bail, Result};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, RgbImage, RgbaImage};

use super::{Frame, ScreenSource};

pub const MAX_WIDTH: u32 = 1600;      // au-delà, l'image est réduite : le débit reste raisonnable sur une connexion domestique
pub const JPEG_QUALITY: u8 = 55;

/// Image de test : dégradé et barre qui se déplace (on voit tout de suite si le flux est vivant).
#[cfg_attr(windows, allow(dead_code))]      // sous Windows, l'écran réel (xcap) la remplace ; elle sert ailleurs et dans les tests
pub struct SyntheticScreen {
    width: u32,
    height: u32,
    tick: u32,
}

#[cfg_attr(windows, allow(dead_code))]
impl SyntheticScreen {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, tick: 0 }
    }
}

#[cfg_attr(windows, allow(dead_code))]
impl ScreenSource for SyntheticScreen {
    fn capture(&mut self) -> Result<Frame> {
        let bar = (self.tick * 16) % self.width.max(1);
        self.tick = self.tick.wrapping_add(1);
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height {
            for x in 0..self.width {
                let on_bar = x >= bar && x < bar + 24;
                let (r, g, b) = if on_bar { (255, 255, 255) } else { ((x * 255 / self.width.max(1)) as u8, (y * 255 / self.height.max(1)) as u8, 96) };
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Ok(Frame { width: self.width, height: self.height, rgba })
    }
}

#[cfg(windows)]
pub struct NativeScreen {
    monitor: xcap::Monitor,
}

#[cfg(windows)]
impl NativeScreen {
    pub fn primary() -> Result<Self> {
        let mut monitors = xcap::Monitor::all()?;
        let index = monitors.iter().position(|m| m.is_primary().unwrap_or(false)).unwrap_or(0);
        if index >= monitors.len() {
            bail!("aucun écran détecté");
        }
        Ok(Self { monitor: monitors.swap_remove(index) })
    }
}

#[cfg(windows)]
impl ScreenSource for NativeScreen {
    fn capture(&mut self) -> Result<Frame> {
        let image = self.monitor.capture_image()?;
        Ok(Frame { width: image.width(), height: image.height(), rgba: image.into_raw() })
    }
}

/// Source d'écran de la plateforme, à créer DANS le thread de capture (certaines API d'écran ne sont pas transférables d'un thread à l'autre).
pub fn default_source() -> Result<Box<dyn ScreenSource>> {
    #[cfg(windows)]
    {
        Ok(Box::new(NativeScreen::primary()?))
    }
    #[cfg(not(windows))]
    {
        Ok(Box::new(SyntheticScreen::new(1280, 720)))
    }
}

/// Réduit si besoin (largeur max) puis compresse en JPEG.
pub fn encode_jpeg(frame: Frame, max_width: u32, quality: u8) -> Result<Vec<u8>> {
    let Some(rgba) = RgbaImage::from_raw(frame.width, frame.height, frame.rgba) else {
        bail!("image brute incohérente avec sa taille");
    };
    let mut image = DynamicImage::ImageRgba8(rgba);
    if image.width() > max_width {
        let height = (u64::from(image.height()) * u64::from(max_width) / u64::from(image.width())).max(1) as u32;
        image = image.resize_exact(max_width, height, FilterType::Triangle);
    }
    let rgb: RgbImage = image.to_rgb8();     // JPEG sans transparence
    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, quality).encode_image(&rgb)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_synthetic_screen_moves_and_encodes_to_a_valid_jpeg() {
        let mut screen = SyntheticScreen::new(320, 180);
        let first = screen.capture().unwrap();
        let second = screen.capture().unwrap();
        assert_eq!((first.width, first.height, first.rgba.len()), (320, 180, 320 * 180 * 4));
        assert_ne!(first.rgba, second.rgba, "la barre doit avoir bougé");
        let jpeg = encode_jpeg(first, MAX_WIDTH, JPEG_QUALITY).unwrap();
        assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF]);
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (320, 180));
    }

    #[test]
    fn wide_screens_are_downscaled_keeping_the_aspect_ratio() {
        let frame = SyntheticScreen::new(3200, 1800).capture().unwrap();
        let decoded = image::load_from_memory(&encode_jpeg(frame, 1600, 40).unwrap()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1600, 900));
    }

    #[test]
    fn an_inconsistent_raw_image_is_refused() {
        assert!(encode_jpeg(Frame { width: 10, height: 10, rgba: vec![0; 5] }, 100, 50).is_err());
    }
}
