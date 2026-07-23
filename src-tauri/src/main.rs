//! Provides the native executable entry point and delegates application setup
//! to the reusable Tauri library crate.
//! Made by Heavymask — https://heavymask.com

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    codex_usage_overlay_lib::run();
}
