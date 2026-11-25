//! Compression H.264 de l'écran (sous Windows : encodeur Media Foundation de Windows). Ce module ne fait QUE l'encodage : le transport RTP, la
//! négociation et le repli JPEG sont dans `session.rs` et `network/webrtc.rs`.
//!
//! Choix (mesurés par le banc d'essai de docs/PLAN_H264.md) : encodeur LOGICIEL synchrone de Windows, profil « Constrained Baseline » (compatible avec tous
//! les décodeurs de navigateur), pas d'images B, débit constant, image clé à la demande. Sortie en Annex B (codes de départ 00 00 00 01), une image par
//! appel, avec SPS/PPS devant chaque image clé.

use anyhow::{anyhow, bail, Result};
use fast_image_resize::images::Image as ResizeImage;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

use super::Frame;

/// Réglages demandés à l'encodeur ; un changement de taille ou d'images par seconde le recrée, un changement de débit s'applique à chaud.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct H264Params {
    pub max_width: u32,
    pub fps: u32,
    pub kbps: u32,
}

/// Une image encodée.
pub struct Encoded {
    /// Annex B : une image complète (SPS, PPS et image clé, ou une seule tranche).
    pub data: Vec<u8>,
    pub keyframe: bool,
}

/// RGBA -> NV12 (BT.601, plage limitée) ; la chrominance est la moyenne de chaque bloc 2x2. `width` et `height` sont pairs.
pub fn rgba_to_nv12(rgba: &[u8], width: usize, height: usize, out: &mut Vec<u8>) {
    debug_assert!(width % 2 == 0 && height % 2 == 0 && rgba.len() >= width * height * 4);
    out.clear();
    out.resize(width * height * 3 / 2, 0);
    let (luma, chroma) = out.split_at_mut(width * height);
    for y in 0..height {
        let row = &rgba[y * width * 4..(y + 1) * width * 4];
        for (x, px) in row.chunks_exact(4).enumerate() {
            let (r, g, b) = (i32::from(px[0]), i32::from(px[1]), i32::from(px[2]));
            luma[y * width + x] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
        }
    }
    for y in (0..height).step_by(2) {
        let (top, bottom) = (&rgba[y * width * 4..], &rgba[(y + 1) * width * 4..]);
        for x in (0..width).step_by(2) {
            let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
            for (row, dx) in [(top, 0), (top, 1), (bottom, 0), (bottom, 1)] {
                let i = (x + dx) * 4;
                r += i32::from(row[i]);
                g += i32::from(row[i + 1]);
                b += i32::from(row[i + 2]);
            }
            let (r, g, b) = ((r + 2) >> 2, (g + 2) >> 2, (b + 2) >> 2);
            let at = (y / 2) * width + x;
            chroma[at] = ((((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255)) as u8;
            chroma[at + 1] = ((((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255)) as u8;
        }
    }
}

/// Réduit si besoin (largeur max, dimensions paires) et convertit en NV12. Renvoie la taille de l'image produite.
fn prepare(resizer: &mut Resizer, frame: Frame, max_width: u32, nv12: &mut Vec<u8>) -> Result<(u32, u32)> {
    let Frame { width, height, rgba } = frame;
    if width < 2 || height < 2 || rgba.len() != (width as usize) * (height as usize) * 4 {
        bail!("image brute incohérente avec sa taille");
    }
    let target_width = width.min(max_width.max(2)) & !1;
    let target_height = ((u64::from(height) * u64::from(target_width) / u64::from(width)).max(2) as u32) & !1;
    if (target_width, target_height) == (width, height) {
        rgba_to_nv12(&rgba, width as usize, height as usize, nv12);
    } else {
        let source = ResizeImage::from_vec_u8(width, height, rgba, PixelType::U8x4).map_err(|e| anyhow!("{e}"))?;
        let mut target = ResizeImage::new(target_width, target_height, PixelType::U8x4);
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
        resizer.resize(&source, &mut target, &options).map_err(|e| anyhow!("{e}"))?;
        rgba_to_nv12(target.buffer(), target_width as usize, target_height as usize, nv12);
    }
    Ok((target_width, target_height))
}

/// Encodeur H.264 d'écran. À créer ET utiliser dans le même thread (COM / Media Foundation).
pub struct H264Encoder {
    resizer: Resizer,
    nv12: Vec<u8>,
    backend: Option<(backend::Backend, (u32, u32, u32))>,   // (encodeur, (largeur, hauteur, images/s)) pour lesquels il a été créé
    kbps: u32,
}

impl H264Encoder {
    /// Vrai si un encodeur H.264 est utilisable sur cette machine (sert à annoncer la fonctionnalité au contrôleur).
    pub fn available() -> bool {
        backend::available()
    }

    pub fn new() -> Self {
        Self { resizer: Resizer::new(), nv12: Vec::new(), backend: None, kbps: 0 }
    }

    /// Encode une image. `Ok(None)` : l'encodeur n'a rien (encore) produit. `force_key` : demande une image clé (le contrôleur a perdu des paquets).
    pub fn encode(&mut self, frame: Frame, params: H264Params, force_key: bool) -> Result<Option<Encoded>> {
        let (width, height) = prepare(&mut self.resizer, frame, params.max_width, &mut self.nv12)?;
        let shape = (width, height, params.fps);
        if self.backend.as_ref().map(|(_, s)| *s) != Some(shape) {
            self.backend = Some((backend::Backend::new(width, height, params.fps, params.kbps)?, shape));
            self.kbps = params.kbps;
        }
        let (backend, _) = self.backend.as_mut().expect("créé ci-dessus");
        if params.kbps != self.kbps {
            backend.set_bitrate(params.kbps)?;
            self.kbps = params.kbps;
        }
        backend.encode(&self.nv12, force_key)
    }
}

/// Repère les images clés d'un flux Annex B : une tranche IDR (type 5) y figure.
pub fn contains_idr(annex_b: &[u8]) -> bool {
    annex_b.windows(4).enumerate().any(|(i, w)| w[..3] == [0, 0, 1] && w[3] & 0x1f == 5 || (w == [0, 0, 0, 1] && annex_b.get(i + 4).is_some_and(|b| b & 0x1f == 5)))
}

#[cfg(windows)]
mod backend {
    use std::mem::ManuallyDrop;
    use std::ptr;

    use anyhow::{anyhow, bail, Context, Result};
    use windows::core::Interface;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
    use windows::Win32::System::Variant::{VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_UI4};

    use super::{contains_idr, Encoded};

    const H264_PROFILE_BASELINE: u32 = 66;
    const RATE_CONTROL_CBR: u32 = 0;

    pub fn available() -> bool {
        // Un petit encodeur de 64 x 64 : s'il se crée, l'ordinateur sait faire du H.264.
        std::thread::spawn(|| Backend::new(64, 64, 10, 200).is_ok()).join().unwrap_or(false)
    }

    pub struct Backend {
        transform: IMFTransform,
        codec: ICodecAPI,
        out_buffer: Option<IMFSample>,   // sortie allouée par nous quand l'encodeur n'en fournit pas
        provides_samples: bool,
        frame: i64,
        duration: i64,                   // en unités de 100 ns
    }

    /// VARIANT « entier 32 bits non signé » (la plupart des réglages d'encodeur de Media Foundation). Rien à libérer pour ce type.
    fn variant_u32(value: u32) -> VARIANT {
        VARIANT {
            Anonymous: VARIANT_0 { Anonymous: ManuallyDrop::new(VARIANT_0_0 { vt: VT_UI4, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: VARIANT_0_0_0 { ulVal: value } }) },
        }
    }

    /// VARIANT booléen (VARIANT_TRUE = -1) : les réglages « faible latence » exigent CE type, pas un entier.
    fn variant_bool(value: bool) -> VARIANT {
        let vt: VARENUM = VT_BOOL;
        VARIANT {
            Anonymous: VARIANT_0 { Anonymous: ManuallyDrop::new(VARIANT_0_0 { vt, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: VARIANT_0_0_0 { boolVal: windows::Win32::Foundation::VARIANT_BOOL(if value { -1 } else { 0 }) } }) },
        }
    }

    fn set_u32(codec: &ICodecAPI, key: &windows::core::GUID, value: u32) -> Result<()> {
        unsafe { codec.SetValue(key, &variant_u32(value)) }.map_err(|e| anyhow!("réglage de l'encodeur refusé : {e}"))
    }

    fn set_bool(codec: &ICodecAPI, key: &windows::core::GUID, value: bool) -> Result<()> {
        unsafe { codec.SetValue(key, &variant_bool(value)) }.map_err(|e| anyhow!("réglage de l'encodeur refusé : {e}"))
    }

    impl Backend {
        pub fn new(width: u32, height: u32, fps: u32, kbps: u32) -> Result<Self> {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_FULL).context("Media Foundation indisponible")?;

                let wanted = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
                let mut list: *mut Option<IMFActivate> = ptr::null_mut();
                let mut count = 0u32;
                MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER, None, Some(&wanted), &mut list, &mut count)
                    .context("recherche d'un encodeur H.264")?;
                if list.is_null() || count == 0 {
                    bail!("aucun encodeur H.264 logiciel sur cet ordinateur");
                }
                let activates: Vec<Option<IMFActivate>> = (0..count as usize).map(|i| ptr::read(list.add(i))).collect();
                CoTaskMemFree(Some(list as *const _));
                let activate = activates.iter().flatten().next().context("encodeur H.264 introuvable")?;
                let transform: IMFTransform = activate.ActivateObject().context("activation de l'encodeur H.264")?;

                let codec: ICodecAPI = transform.cast().context("l'encodeur n'expose pas ICodecAPI")?;
                // Sans ces réglages, l'encodeur de Windows garde ~16 images en réserve avant de sortir la première (1,6 s à 10 images/s).
                set_bool(&codec, &CODECAPI_AVLowLatencyMode, true).context("mode faible latence refusé")?;
                let _ = set_bool(&codec, &CODECAPI_AVEncCommonRealTime, true);
                let _ = set_u32(&codec, &CODECAPI_AVEncMPVDefaultBPictureCount, 0);
                if let Ok(attributes) = transform.GetAttributes() {
                    let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
                }
                set_u32(&codec, &CODECAPI_AVEncCommonRateControlMode, RATE_CONTROL_CBR)?;
                set_u32(&codec, &CODECAPI_AVEncCommonMeanBitRate, kbps * 1000)?;
                let _ = set_u32(&codec, &CODECAPI_AVEncMPVGOPSize, fps * 2);      // image clé toutes les ~2 s en plus de celles demandées

                let output = MFCreateMediaType()?;
                output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                output.SetUINT32(&MF_MT_AVG_BITRATE, kbps * 1000)?;
                output.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(width) << 32) | u64::from(height))?;
                output.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(fps) << 32) | 1)?;
                output.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
                output.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                output.SetUINT32(&MF_MT_MPEG2_PROFILE, H264_PROFILE_BASELINE)?;
                transform.SetOutputType(0, &output, 0).context("type de sortie H.264 refusé")?;

                let input = MFCreateMediaType()?;
                input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                input.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(width) << 32) | u64::from(height))?;
                input.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(fps) << 32) | 1)?;
                input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
                input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                transform.SetInputType(0, &input, 0).context("type d'entrée NV12 refusé")?;

                transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
                transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;

                let info = transform.GetOutputStreamInfo(0)?;
                let provides_samples = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;
                let out_buffer = if provides_samples {
                    None
                } else {
                    let sample = MFCreateSample()?;
                    sample.AddBuffer(&MFCreateMemoryBuffer(info.cbSize.max(1 << 20))?)?;
                    Some(sample)
                };
                Ok(Self { transform, codec, out_buffer, provides_samples, frame: 0, duration: 10_000_000 / i64::from(fps.max(1)) })
            }
        }

        pub fn set_bitrate(&mut self, kbps: u32) -> Result<()> {
            set_u32(&self.codec, &CODECAPI_AVEncCommonMeanBitRate, kbps * 1000)
        }

        pub fn encode(&mut self, nv12: &[u8], force_key: bool) -> Result<Option<Encoded>> {
            unsafe {
                if force_key {
                    set_u32(&self.codec, &CODECAPI_AVEncVideoForceKeyFrame, 1)?;
                }
                let buffer = MFCreateMemoryBuffer(nv12.len() as u32)?;
                let mut data: *mut u8 = ptr::null_mut();
                buffer.Lock(&mut data, None, None)?;
                ptr::copy_nonoverlapping(nv12.as_ptr(), data, nv12.len());
                buffer.Unlock()?;
                buffer.SetCurrentLength(nv12.len() as u32)?;
                let sample = MFCreateSample()?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(self.frame * self.duration)?;
                sample.SetSampleDuration(self.duration)?;
                self.frame += 1;
                self.transform.ProcessInput(0, &sample, 0)?;

                let mut produced = None;
                loop {
                    let mut output = [MFT_OUTPUT_DATA_BUFFER {
                        dwStreamID: 0,
                        pSample: ManuallyDrop::new(if self.provides_samples { None } else { self.out_buffer.clone() }),
                        dwStatus: 0,
                        pEvents: ManuallyDrop::new(None),
                    }];
                    let mut status = 0u32;
                    let result = self.transform.ProcessOutput(0, &mut output, &mut status);
                    let sample = ManuallyDrop::take(&mut output[0].pSample);
                    let _events = ManuallyDrop::take(&mut output[0].pEvents);
                    match result {
                        Ok(()) => {
                            if let Some(sample) = sample {
                                let buffer = sample.ConvertToContiguousBuffer()?;
                                let (mut data, mut len) = (ptr::null_mut(), 0u32);
                                buffer.Lock(&mut data, None, Some(&mut len))?;
                                let bytes = std::slice::from_raw_parts(data, len as usize).to_vec();
                                buffer.Unlock()?;
                                if !bytes.is_empty() {
                                    produced = Some(bytes);   // une seule image par entrée : on garde la dernière sortie
                                }
                            }
                        }
                        Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                        Err(error) => bail!("encodage H.264 : {error}"),
                    }
                }
                Ok(produced.map(|data| Encoded { keyframe: contains_idr(&data), data }))
            }
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            unsafe {
                let _ = self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
                let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            }
        }
    }
}

#[cfg(not(windows))]
mod backend {
    use anyhow::{bail, Result};

    use super::Encoded;

    pub fn available() -> bool {
        false
    }

    pub struct Backend;

    impl Backend {
        pub fn new(_width: u32, _height: u32, _fps: u32, _kbps: u32) -> Result<Self> {
            bail!("H.264 : encodeur disponible seulement sous Windows")
        }
        pub fn set_bitrate(&mut self, _kbps: u32) -> Result<()> {
            Ok(())
        }
        pub fn encode(&mut self, _nv12: &[u8], _force_key: bool) -> Result<Option<Encoded>> {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::screen::SyntheticScreen;
    #[cfg(windows)]
    use crate::capture::Frame;
    use crate::capture::ScreenSource;

    #[test]
    fn nv12_has_the_expected_size_and_known_colors() {
        // 2 x 2 de blanc puis de noir : luminance 235 / 16, chrominance neutre (128).
        let mut out = Vec::new();
        rgba_to_nv12(&[255; 16], 2, 2, &mut out);
        assert_eq!(out.len(), 6);
        assert_eq!(&out[..4], &[235; 4]);
        assert_eq!(&out[4..], &[128, 128]);
        rgba_to_nv12(&[0, 0, 0, 255].repeat(4), 2, 2, &mut out);
        assert_eq!(&out[..4], &[16; 4]);
    }

    #[test]
    fn idr_detection_handles_both_start_code_lengths() {
        assert!(contains_idr(&[0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x65, 9]));
        assert!(contains_idr(&[0, 0, 1, 0x65, 9]));
        assert!(!contains_idr(&[0, 0, 0, 1, 0x41, 9, 9]));
    }

    #[test]
    #[cfg(windows)]
    fn the_windows_encoder_produces_a_keyframe_then_small_frames() {
        assert!(H264Encoder::available(), "pas d'encodeur H.264 sur cette machine");
        let mut screen = SyntheticScreen::new(1280, 720);
        let mut encoder = H264Encoder::new();
        let params = H264Params { max_width: 1280, fps: 10, kbps: 1500 };
        let (mut keys, mut total, mut first_is_key) = (0, 0usize, None);
        for n in 0..40 {
            let frame = screen.capture().unwrap();
            if let Some(encoded) = encoder.encode(frame, params, n == 25).unwrap() {
                if first_is_key.is_none() {
                    assert!(n <= 1, "latence d'encodage : première sortie seulement à l'entrée n° {n} (mode faible latence non appliqué ?)");
                }
                first_is_key.get_or_insert(encoded.keyframe);
                assert_eq!(&encoded.data[..4], &[0, 0, 0, 1], "Annex B attendu");
                keys += usize::from(encoded.keyframe);
                total += encoded.data.len();
            }
        }
        assert_eq!(first_is_key, Some(true), "la première image doit être une image clé");
        assert!(keys >= 2, "image clé demandée à la 25e image : {keys} images clés vues");
        assert!(total > 0 && total < 40 * 60_000, "volume incohérent : {total} octets");
    }

    #[test]
    #[cfg(windows)]
    fn a_new_size_recreates_the_encoder_and_a_new_bitrate_applies_live() {
        let mut encoder = H264Encoder::new();
        for (width, kbps) in [(1280u32, 1000u32), (1280, 2000), (960, 2000)] {
            let mut screen = SyntheticScreen::new(1280, 720);
            let mut seen = 0;
            for _ in 0..5 {
                let params = H264Params { max_width: width, fps: 10, kbps };
                seen += usize::from(encoder.encode(screen.capture().unwrap(), params, false).unwrap().is_some());
            }
            assert!(seen > 0, "aucune image pour {width} px / {kbps} kbit/s");
        }
    }

    /// Image pleine de bruit qui change à chaque fois : le plus dur à compresser, donc l'encodeur est limité par son débit cible, pas par le contenu.
    #[cfg(windows)]
    fn noisy(width: u32, height: u32, seed: u32) -> Frame {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(12345);
        let rgba = (0..width * height * 4)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        Frame { width, height, rgba }
    }

    #[test]
    #[cfg(windows)]
    fn a_lower_bitrate_request_really_lowers_the_output_while_streaming() {
        let mut encoder = H264Encoder::new();
        let mut volume = |kbps: u32, first: u32| {
            let params = H264Params { max_width: 960, fps: 10, kbps };
            let mut total = 0usize;
            for n in 0..40 {
                let encoded = encoder.encode(noisy(960, 540, first + n), params, false).unwrap();
                if n >= 10 {
                    total += encoded.map_or(0, |e| e.data.len());      // on laisse l'encodeur se stabiliser
                }
            }
            total * 8 / 1000 / 3          // kbit/s sur 30 images à 10 images/s
        };
        let high = volume(3000, 0);
        let low = volume(500, 100);
        println!("débit demandé 3000 -> {high} kbit/s ; 500 -> {low} kbit/s");
        assert!(low * 2 < high, "le débit demandé n'est pas suivi : {high} kbit/s puis {low} kbit/s");
    }

    #[test]
    #[ignore = "mesure, pas un test : à lancer à la main"]
    #[cfg(windows)]
    fn encode_speed_on_this_machine() {
        let mut screen = SyntheticScreen::new(1920, 1080);
        let mut encoder = H264Encoder::new();
        let params = H264Params { max_width: 1600, fps: 10, kbps: 2000 };
        let (mut bytes, started) = (0usize, std::time::Instant::now());
        for _ in 0..60 {
            bytes += encoder.encode(screen.capture().unwrap(), params, false).unwrap().map_or(0, |e| e.data.len());
        }
        println!("1920x1080 -> 1600 px : {:.1} ms par image, {} Ko au total", started.elapsed().as_secs_f64() * 1000.0 / 60.0, bytes / 1024);
    }
}
