// Pas de console derrière la fenêtre sous Windows en version release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    aria_remote_desktop_lib::run()
}
