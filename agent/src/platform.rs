//! Type de la machine (PC de bureau, mini PC, portable), annoncé au serveur pour que le contrôleur affiche le bon dessin.
//!
//! Source : le « type de châssis » du BIOS (SMBIOS, la même valeur sur tous les systèmes). Quand le BIOS ne le dit pas (machine virtuelle,
//! valeurs 1 et 2), la présence d'une batterie départage : batterie = portable. Le résultat est une étiquette courte : « windows » (bureau),
//! « windows-laptop » ou « windows-mini » (même forme que `platform` côté serveur : minuscules, chiffres, tirets, 2 à 20 caractères).
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Desktop,
    Mini,
    Laptop,
}

/// Valeurs SMBIOS « System Enclosure » : https://www.dmtf.org/standards/smbios (champ Type du boîtier).
pub fn kind_from_chassis(code: u32) -> Option<Kind> {
    match code {
        8 | 9 | 10 | 11 | 14 | 30 | 31 | 32 => Some(Kind::Laptop),       // portable, laptop, notebook, handheld, sub-notebook, tablette, convertible, détachable
        34 | 35 | 36 => Some(Kind::Mini),                                  // embedded PC, mini PC, stick PC
        3 | 4 | 5 | 6 | 7 | 13 | 15 | 16 | 17 | 23 => Some(Kind::Desktop), // bureau, tours, tout-en-un, serveurs
        _ => None,                                                         // 1 « autre », 2 « inconnu », et le reste
    }
}

/// Premier type de châssis connu ; à défaut, la batterie : présente = portable, absente = bureau.
pub fn decide(chassis: &[u32], has_battery: bool) -> Kind {
    chassis.iter().find_map(|code| kind_from_chassis(*code)).unwrap_or(if has_battery { Kind::Laptop } else { Kind::Desktop })
}

pub fn label(os: &str, kind: Kind) -> String {
    match kind {
        Kind::Desktop => os.to_string(),
        Kind::Mini => format!("{os}-mini"),
        Kind::Laptop => format!("{os}-laptop"),
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
/// Sortie de la commande PowerShell de Windows : « 9,10|1 » = types de châssis | nombre de batteries.
pub fn parse_windows(output: &str) -> (Vec<u32>, bool) {
    let line = output.lines().map(str::trim).find(|l| l.contains('|')).unwrap_or("");
    let (chassis, batteries) = line.split_once('|').unwrap_or(("", "0"));
    (chassis.split(',').filter_map(|c| c.trim().parse().ok()).collect(), batteries.trim().parse::<u32>().unwrap_or(0) > 0)
}

/// Macs : le modèle (« MacBookPro18,3 », « Macmini9,1 »...) dit tout.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn kind_from_mac_model(model: &str) -> Kind {
    let model = model.to_ascii_lowercase();
    if model.contains("book") { Kind::Laptop } else if model.contains("mini") { Kind::Mini } else { Kind::Desktop }
}

#[cfg(windows)]
fn probe() -> Kind {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = "$t = @((Get-CimInstance Win32_SystemEnclosure).ChassisTypes) -join ','; $b = @(Get-CimInstance Win32_Battery).Count; \"$t|$b\"";
    let output = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]).creation_flags(CREATE_NO_WINDOW).output();
    match output {
        Ok(out) if out.status.success() => {
            let (chassis, battery) = parse_windows(&String::from_utf8_lossy(&out.stdout));
            decide(&chassis, battery)
        }
        _ => Kind::Desktop,
    }
}

#[cfg(target_os = "linux")]
fn probe() -> Kind {
    let chassis: Vec<u32> = std::fs::read_to_string("/sys/class/dmi/id/chassis_type").ok().and_then(|t| t.trim().parse().ok()).into_iter().collect();
    let battery = std::fs::read_dir("/sys/class/power_supply").map(|dir| dir.flatten().any(|e| e.file_name().to_string_lossy().starts_with("BAT"))).unwrap_or(false);
    decide(&chassis, battery)
}

#[cfg(target_os = "macos")]
fn probe() -> Kind {
    std::process::Command::new("sysctl").args(["-n", "hw.model"]).output().ok().map(|o| kind_from_mac_model(&String::from_utf8_lossy(&o.stdout))).unwrap_or(Kind::Desktop)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn probe() -> Kind {
    Kind::Desktop
}

/// Étiquette de cette machine, sans jamais échouer : en cas de doute, c'est un PC de bureau. Peut prendre une seconde sous Windows (PowerShell) :
/// à appeler hors du fil principal.
pub fn detect() -> String {
    label(std::env::consts::OS, probe())
}

/// `detect` dans un fil à part, avec un délai : un PowerShell bloqué ne doit jamais empêcher l'agent de démarrer.
pub async fn detect_in_background() -> String {
    let default = std::env::consts::OS.to_string();
    match tokio::time::timeout(Duration::from_secs(10), tokio::task::spawn_blocking(detect)).await {
        Ok(Ok(found)) => found,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chassis_codes_map_to_the_three_kinds() {
        for code in [8, 9, 10, 14, 30, 31, 32] { assert_eq!(kind_from_chassis(code), Some(Kind::Laptop), "{code}"); }
        for code in [34, 35, 36] { assert_eq!(kind_from_chassis(code), Some(Kind::Mini), "{code}"); }
        for code in [3, 4, 6, 7, 13, 15] { assert_eq!(kind_from_chassis(code), Some(Kind::Desktop), "{code}"); }
        for code in [0, 1, 2, 99] { assert_eq!(kind_from_chassis(code), None, "{code}"); }
    }

    #[test]
    fn an_unknown_chassis_falls_back_to_the_battery() {
        assert_eq!(decide(&[], true), Kind::Laptop);
        assert_eq!(decide(&[], false), Kind::Desktop);
        assert_eq!(decide(&[2], true), Kind::Laptop);           // machine virtuelle ou BIOS muet
        assert_eq!(decide(&[1, 35], false), Kind::Mini);        // le premier type connu gagne
        assert_eq!(decide(&[3], true), Kind::Desktop);          // un bureau avec onduleur détecté comme batterie reste un bureau
        assert_eq!(decide(&[10], false), Kind::Laptop);         // un portable batterie retirée reste un portable
    }

    #[test]
    fn labels_have_the_shape_the_server_accepts() {
        assert_eq!(label("windows", Kind::Desktop), "windows");
        assert_eq!(label("windows", Kind::Laptop), "windows-laptop");
        assert_eq!(label("windows", Kind::Mini), "windows-mini");
        for os in ["windows", "linux", "macos"] {
            for kind in [Kind::Desktop, Kind::Mini, Kind::Laptop] {
                let text = label(os, kind);
                assert!((2..=20).contains(&text.len()) && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'), "{text}");
            }
        }
    }

    #[test]
    fn windows_output_is_parsed_even_with_noise() {
        assert_eq!(parse_windows("9,10|1\r\n"), (vec![9, 10], true));
        assert_eq!(parse_windows("avertissement\n3|0\n"), (vec![3], false));
        assert_eq!(parse_windows("|0"), (vec![], false));
        assert_eq!(parse_windows(""), (vec![], false));
        assert_eq!(parse_windows("n'importe quoi"), (vec![], false));
    }

    #[test]
    fn mac_models_are_recognized() {
        assert_eq!(kind_from_mac_model("MacBookPro18,3\n"), Kind::Laptop);
        assert_eq!(kind_from_mac_model("MacBookAir10,1"), Kind::Laptop);
        assert_eq!(kind_from_mac_model("Macmini9,1"), Kind::Mini);
        assert_eq!(kind_from_mac_model("iMac21,1"), Kind::Desktop);
    }

    #[tokio::test]
    async fn detection_never_fails_and_always_gives_a_valid_label() {
        let found = detect_in_background().await;
        assert!(found.starts_with(std::env::consts::OS), "{found}");
    }
}
