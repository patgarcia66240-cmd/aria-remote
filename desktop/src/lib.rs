//! ARIA Remote Desktop : le contrôleur autonome.
//!
//! Étape actuelle : squelette. La fenêtre s'ouvre et affiche la version ; la liste des appareils, l'appairage par code et l'écran
//! distant viennent ensuite (voir README.md, « Feuille de route »).

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_version])
        .run(tauri::generate_context!())
        .expect("échec du démarrage d'ARIA Remote Desktop");
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_cargo_version() {
        assert_eq!(super::app_version(), env!("CARGO_PKG_VERSION"));
    }
}
