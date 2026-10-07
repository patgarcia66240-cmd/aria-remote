//! Identité de l'exécutable Windows : icône, nom du produit, description, version. Windows affiche ces informations dans l'explorateur, le gestionnaire de
//! tâches et la fenêtre d'avertissement SmartScreen (« Éditeur inconnu » reste affiché tant que l'exe n'est pas signé, voir README.md).
//! Compilé et exécuté seulement quand on construit sous Windows ; ailleurs, ce script ne fait rien.

#[cfg(windows)]
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icon.ico");
    resource.set("ProductName", "ARIA Remote");
    resource.set("FileDescription", "Agent Remote de PC Assistant (ARIA)");
    resource.set("CompanyName", "ARIA PC Assistant");
    resource.set("LegalCopyright", "© 2026 ARIA PC Assistant");
    resource.set("OriginalFilename", "remote-agent.exe");
    // La version vient de Cargo.toml (CARGO_PKG_VERSION). Un échec ici ne doit jamais empêcher de produire l'exe : on le signale seulement.
    if let Err(error) = resource.compile() {
        println!("cargo:warning=identité de l'exe non intégrée : {error}");
    }
}

#[cfg(not(windows))]
fn main() {}
