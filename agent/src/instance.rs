//! Une seule instance de l'agent par appareil : deux agents avec la même identité se disputeraient la connexion au serveur et les demandes d'accord.
//! Verrou de fichier à côté de la configuration : le système le libère tout seul quand le processus s'arrête, même planté (jamais de verrou resté).

use std::fs::{self, File, TryLockError};
use std::path::Path;

use anyhow::{bail, Context, Result};

/// Tenir cette valeur garde le verrou ; la lâcher (ou finir le processus) le libère.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

pub fn acquire(config_path: &Path) -> Result<Lock> {
    let path = config_path.with_extension("lock");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("dossier {} impossible à créer", dir.display()))?;
    }
    let file = File::options().create(true).truncate(false).write(true).open(&path).with_context(|| format!("{} illisible", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Lock { _file: file }),
        Err(TryLockError::WouldBlock) => bail!("L'agent Remote tourne déjà sur cet appareil (une seule instance à la fois : ARIA Remote ou remote-agent.exe)."),
        Err(TryLockError::Error(error)) => Err(error).with_context(|| format!("verrou {} impossible", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_instance_is_refused_until_the_first_one_stops() {
        let dir = std::env::temp_dir().join(format!("aria-remote-lock-{}", std::process::id()));
        let config = dir.join("config.json");
        let first = acquire(&config).expect("premier verrou");
        let refused = acquire(&config).expect_err("second verrou refusé").to_string();
        assert!(refused.contains("tourne déjà"), "{refused}");
        drop(first);
        acquire(&config).expect("verrou libéré après l'arrêt de la première instance");
        let _ = fs::remove_dir_all(&dir);
    }
}
