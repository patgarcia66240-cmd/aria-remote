//! Démarrage avec Windows : une valeur dans la clé « Run » du profil de l'utilisateur (aucun droit administrateur). Ailleurs : sans effet.
//! Le programme démarre alors avec `--minimized` : il reste près de l'horloge sans ouvrir de fenêtre.

pub const FLAG: &str = "--minimized";

#[cfg(windows)]
pub fn set(enabled: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const NO_WINDOW: u32 = 0x0800_0000;
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let mut command = std::process::Command::new("reg");
    if enabled {
        let exe = std::env::current_exe().map_err(|e| format!("Programme introuvable : {e}"))?;
        command.args(["add", KEY, "/v", "ARIARemote", "/t", "REG_SZ", "/d", &format!("\"{}\" {FLAG}", exe.display()), "/f"]);
    } else {
        command.args(["delete", KEY, "/v", "ARIARemote", "/f"]);
    }
    let output = command.creation_flags(NO_WINDOW).output().map_err(|e| format!("Démarrage automatique impossible : {e}"))?;
    // Supprimer une valeur qui n'existe pas n'est pas une erreur.
    if output.status.success() || !enabled { Ok(()) } else { Err("Démarrage automatique impossible (écriture du registre refusée).".into()) }
}

#[cfg(not(windows))]
pub fn set(_enabled: bool) -> Result<(), String> {
    Ok(())
}

pub fn started_minimized() -> bool {
    std::env::args().any(|arg| arg == FLAG)
}
