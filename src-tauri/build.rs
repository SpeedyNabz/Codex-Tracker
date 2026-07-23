//! Runs Tauri's build-time integration so the native application context and
//! generated resources are prepared before compiling the backend.
//! Made by Heavymask — https://heavymask.com

fn main() {
    tauri_build::build()
}
