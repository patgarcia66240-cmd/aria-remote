//! Journal d'événements de l'agent, écrit dans un fichier pour pouvoir relire une session après coup.
//!
//! Fichier : `agent.log` dans le dossier de configuration (`%APPDATA%\ARIA Remote` sous Windows).
//! Une ligne par événement : date UTC, puis le message. Aucun code d'appairage ni clé n'est écrit ici.
//! Écrire dans le journal ne doit jamais faire échouer l'agent : les erreurs sont ignorées.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Dossier qui contient `agent.log`.
fn directory() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(base).join("ARIA Remote"))
}

/// Ajoute une ligne horodatée au journal. Ne fait jamais échouer l'appelant.
pub fn log(message: &str) {
    let Some(dir) = directory() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("agent.log")) {
        let _ = writeln!(file, "{} {}", utc_timestamp(seconds), message);
    }
}

/// Date UTC lisible (AAAA-MM-JJ HH:MM:SS) à partir de secondes depuis 1970.
pub fn utc_timestamp(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let rest = unix_seconds % 86_400;
    // Algorithme de Howard Hinnant (jours civils depuis 1970-01-01).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::utc_timestamp;

    #[test]
    fn timestamps_are_correct_utc_dates() {
        assert_eq!(utc_timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(utc_timestamp(1_793_000_000), "2026-10-26 07:33:20");
    }
}
