//! Sources d'écran et compression JPEG. Windows : écran principal via xcap. Ailleurs (développement, tests) : image synthétique animée.

use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use fast_image_resize::images::Image as ResizeImage;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

use super::{Frame, Screen, ScreenSource, Screens};

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

/// Tous les écrans, triés de gauche à droite puis de haut en bas : le rang d'un écran est stable pendant la session.
#[cfg(windows)]
fn native_monitors() -> Vec<(xcap::Monitor, Screen)> {
    let mut found: Vec<(xcap::Monitor, Screen)> = xcap::Monitor::all()
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            let screen = Screen {
                index: 0,
                name: m.friendly_name().or_else(|_| m.name()).unwrap_or_else(|_| "Écran".to_string()),
                x: m.x().unwrap_or(0),
                y: m.y().unwrap_or(0),
                width: m.width().unwrap_or(0),
                height: m.height().unwrap_or(0),
                primary: m.is_primary().unwrap_or(false),
            };
            (m, screen)
        })
        .filter(|(_, screen)| screen.width > 0 && screen.height > 0)
        .collect();
    found.sort_by_key(|(_, screen)| (screen.x, screen.y));
    for (rank, (_, screen)) in found.iter_mut().enumerate() {
        screen.index = rank;
    }
    found
}

#[cfg(windows)]
impl ScreenSource for NativeScreen {
    fn capture(&mut self) -> Result<Frame> {
        let image = self.monitor.capture_image()?;
        Ok(Frame { width: image.width(), height: image.height(), rgba: image.into_raw() })
    }
}

/// Deux écrans de test (1280x720 principal, 800x600 à sa droite) : de quoi essayer le choix d'écran sans deuxième moniteur.
#[cfg_attr(windows, allow(dead_code))]
pub fn synthetic_screens() -> Vec<Screen> {
    vec![
        Screen { index: 0, name: "Écran de test 1".into(), x: 0, y: 0, width: 1280, height: 720, primary: true },
        Screen { index: 1, name: "Écran de test 2".into(), x: 1280, y: 0, width: 800, height: 600, primary: false },
    ]
}

/// Les écrans de la plateforme.
pub fn default_screens() -> Screens {
    #[cfg(windows)]
    {
        Screens {
            list: Arc::new(|| native_monitors().into_iter().map(|(_, screen)| screen).collect()),
            open: Arc::new(|index| {
                let mut monitors = native_monitors();
                if index >= monitors.len() {
                    bail!("écran {index} introuvable");
                }
                Ok(Box::new(NativeScreen { monitor: monitors.swap_remove(index).0 }) as Box<dyn ScreenSource>)
            }),
        }
    }
    #[cfg(not(windows))]
    {
        synthetic_backend()
    }
}

/// Les deux écrans de test comme « machine » complète (liste + ouverture). Les tests s'en servent TOUJOURS, jamais des vrais écrans :
/// sous Windows, `default_screens` liste ceux de la machine (un seul, souvent, sur un serveur d'intégration).
#[cfg_attr(windows, allow(dead_code))]
pub fn synthetic_backend() -> Screens {
    Screens {
        list: Arc::new(synthetic_screens),
        open: Arc::new(|index| {
            let screens = synthetic_screens();
            let Some(screen) = screens.get(index) else { bail!("écran {index} introuvable") };
            Ok(Box::new(SyntheticScreen::new(screen.width, screen.height)) as Box<dyn ScreenSource>)
        }),
    }
}

/// Réduit si besoin (largeur max) puis compresse en JPEG. Garde son outil de mise à l'échelle d'une image à l'autre (il réutilise sa mémoire).
///
/// Pourquoi ces choix : sur un écran 1080p, l'ancien chemin (mise à l'échelle « image » + encodeur « image ») coûtait ~120 ms par image, soit
/// moins de 8 images/s sur un coeur AVANT même l'envoi ; celui-ci en prend ~18 ms (mise à l'échelle SIMD, encodeur JPEG à instructions vectorielles,
/// chrominance en 4:2:0, pas de conversion RGBA -> RGB intermédiaire).
pub struct JpegEncoder {
    resizer: Resizer,
}

impl JpegEncoder {
    pub fn new() -> Self {
        Self { resizer: Resizer::new() }
    }

    pub fn encode(&mut self, frame: Frame, max_width: u32, quality: u8) -> Result<Vec<u8>> {
        let Frame { width, height, rgba } = frame;
        if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
            bail!("image brute incohérente avec sa taille");
        }
        let (data, out_width, out_height) = if width > max_width {
            let out_height = (u64::from(height) * u64::from(max_width) / u64::from(width)).max(1) as u32;
            let source = ResizeImage::from_vec_u8(width, height, rgba, PixelType::U8x4).map_err(|e| anyhow!("{e}"))?;
            let mut target = ResizeImage::new(max_width, out_height, PixelType::U8x4);
            let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
            self.resizer.resize(&source, &mut target, &options).map_err(|e| anyhow!("{e}"))?;
            (target.into_vec(), max_width, out_height)
        } else {
            (rgba, width, height)
        };
        let (Ok(w), Ok(h)) = (u16::try_from(out_width), u16::try_from(out_height)) else { bail!("écran trop grand pour le JPEG") };
        let mut out = Vec::with_capacity(data.len() / 12);
        let mut encoder = Encoder::new(&mut out, quality);
        encoder.set_sampling_factor(SamplingFactor::F_2_2);
        encoder.encode(&data, w, h, ColorType::Rgba).map_err(|e| anyhow!("{e}"))?;
        Ok(out)
    }
}

/// Raccourci pour les tests et les usages ponctuels.
#[cfg_attr(not(test), allow(dead_code))]
pub fn encode_jpeg(frame: Frame, max_width: u32, quality: u8) -> Result<Vec<u8>> {
    JpegEncoder::new().encode(frame, max_width, quality)
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

    /// Mesure sur CETTE machine : `cargo test --release encode_speed -- --ignored --nocapture`. Avant la refonte, 1600 px coûtait ~120 ms par image.
    #[test]
    #[ignore = "mesure, pas un test : à lancer à la main"]
    fn encode_speed_on_this_machine() {
        let (width, height) = (1920u32, 1080u32);
        let mut rgba = vec![255u8; (width * height * 4) as usize];
        let mut seed = 7u32;
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let (x, y) = ((i as u32) % width, (i as u32) / width);
            let text = (y / 9) % 2 == 0 && (x / 7 + y / 9) % 5 != 0;
            let base = if text { 40 } else { 245 };
            px[..3].fill(if (seed >> 24) > 250 { base / 2 } else { base });
        }
        let mut encoder = JpegEncoder::new();
        for max_width in [1600u32, 1024, 2560] {
            let frame = || Frame { width, height, rgba: rgba.clone() };
            let _ = encoder.encode(frame(), max_width, 55).unwrap();
            let runs = 10;
            let started = std::time::Instant::now();
            let mut size = 0;
            for _ in 0..runs { size = encoder.encode(frame(), max_width, 55).unwrap().len(); }
            let ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
            println!("1920x1080 -> {max_width} px : {ms:.1} ms par image, {} Ko", size / 1024);
        }
    }

    #[test]
    fn an_inconsistent_raw_image_is_refused() {
        assert!(encode_jpeg(Frame { width: 10, height: 10, rgba: vec![0; 5] }, 100, 50).is_err());
    }
}
