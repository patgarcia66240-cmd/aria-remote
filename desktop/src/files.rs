//! Enregistrement des captures d'écran : nom de fichier sûr, décodage et dossier de destination. Aucune dépendance à Tauri.
use std::path::{Path, PathBuf};

use base64::Engine;

/// Taille maximale d'une capture acceptée (30 Mo) : un écran 4K en PNG fait quelques Mo.
pub const MAX_BYTES: usize = 30 * 1024 * 1024;
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Date et heure UTC « AAAAMMJJ-HHMMSS » d'un instant Unix (algorithme du calendrier grégorien civil, sans dépendance).
pub fn timestamp(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let rest = unix_seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}{month:02}{day:02}-{:02}{:02}{:02}", rest / 3_600, rest % 3_600 / 60, rest % 60)
}

/// Nom du fichier : « ARIA-Remote-<appareil>-<date>.png ». Le nom de l'appareil ne garde que lettres, chiffres, « - » et « _ » (jamais de chemin).
pub fn capture_filename(device: &str, unix_seconds: u64) -> String {
    let mut label: String = device.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(24).collect();
    label = label.trim_matches('_').to_string();
    if label.is_empty() {
        label = "appareil".to_string();
    }
    format!("ARIA-Remote-{label}-{}.png", timestamp(unix_seconds))
}

/// Décode une capture envoyée par la fenêtre (base64) et vérifie que c'est bien une image PNG de taille raisonnable.
pub fn decode_png(base64_text: &str) -> Result<Vec<u8>, String> {
    if base64_text.len() > MAX_BYTES * 4 / 3 + 16 {
        return Err("Capture trop volumineuse.".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64_text.trim()).map_err(|_| "Capture illisible.".to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Capture trop volumineuse.".into());
    }
    if !bytes.starts_with(&PNG_MAGIC) {
        return Err("Ce n'est pas une image PNG.".into());
    }
    Ok(bytes)
}

/// Dossier des captures : « Images » de l'utilisateur, sinon son dossier personnel, sinon le dossier courant.
pub fn captures_dir() -> PathBuf {
    dirs::picture_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

/// Écrit la capture dans `dir` sans jamais écraser un fichier existant (« -2 », « -3 »... ajoutés si le nom est pris) ; renvoie le chemin.
pub fn save_png(dir: &Path, filename: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Dossier des captures inaccessible : {e}"))?;
    let stem = filename.trim_end_matches(".png");
    for attempt in 1..=99 {
        let name = if attempt == 1 { filename.to_string() } else { format!("{stem}-{attempt}.png") };
        let path = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(bytes).map_err(|e| format!("Enregistrement impossible : {e}"))?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Enregistrement impossible : {error}")),
        }
    }
    Err("Trop de captures portent déjà ce nom.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_correct_utc_dates() {
        assert_eq!(timestamp(0), "19700101-000000");
        assert_eq!(timestamp(951_782_400), "20000229-000000", "29 février d'une année bissextile (2000)");
        assert_eq!(timestamp(1_791_414_245), "20261007-230405");
        assert_eq!(timestamp(4_102_444_799), "20991231-235959");
        assert_eq!(timestamp(1_709_251_199), "20240229-235959");
    }

    #[test]
    fn the_device_name_can_never_escape_the_folder() {
        assert_eq!(capture_filename("XENPRO", 0), "ARIA-Remote-XENPRO-19700101-000000.png");
        assert_eq!(capture_filename("PC du salon", 0), "ARIA-Remote-PC_du_salon-19700101-000000.png");
        let nasty = capture_filename("../../Windows\\System32\\x", 0);
        assert!(!nasty.contains('/') && !nasty.contains('\\') && !nasty.contains(".."), "{nasty}");
        assert_eq!(capture_filename("", 0), "ARIA-Remote-appareil-19700101-000000.png");
        assert_eq!(capture_filename("é à ç", 0), "ARIA-Remote-appareil-19700101-000000.png");
        assert!(capture_filename(&"a".repeat(200), 0).len() < 60);
    }

    #[test]
    fn only_real_png_images_are_accepted() {
        let png = [PNG_MAGIC.as_slice(), &[1, 2, 3]].concat();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
        assert_eq!(decode_png(&encoded).unwrap(), png);
        assert_eq!(decode_png(&format!("  {encoded}\n")).unwrap(), png, "les espaces autour sont tolérés");
        assert_eq!(decode_png("pas du base64 !!").unwrap_err(), "Capture illisible.");
        assert_eq!(decode_png(&base64::engine::general_purpose::STANDARD.encode(b"MZ\x90 un exe")).unwrap_err(), "Ce n'est pas une image PNG.");
        assert_eq!(decode_png(&"A".repeat(MAX_BYTES * 2)).unwrap_err(), "Capture trop volumineuse.");
    }

    #[test]
    fn a_capture_never_overwrites_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("aria-remote-captures-{}", std::process::id()));
        let first = save_png(&dir, "a.png", b"1").unwrap();
        let second = save_png(&dir, "a.png", b"2").unwrap();
        let third = save_png(&dir, "a.png", b"3").unwrap();
        assert_eq!([first.file_name().unwrap(), second.file_name().unwrap(), third.file_name().unwrap()], ["a.png", "a-2.png", "a-3.png"]);
        assert_eq!(std::fs::read(&first).unwrap(), b"1");
        assert_eq!(std::fs::read(&second).unwrap(), b"2");
        std::fs::remove_dir_all(dir).ok();
    }
}
