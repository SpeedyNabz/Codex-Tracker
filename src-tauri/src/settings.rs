use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub codex_executable: Option<String>,
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub expanded: bool,
    pub autostart_initialized: bool,
}

pub struct LoadedSettings {
    pub value: Settings,
    pub path: PathBuf,
}

pub fn load(app: &AppHandle) -> LoadedSettings {
    let base = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("codex-usage-overlay"));
    let path = base.join("settings.json");
    let value = fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default();
    LoadedSettings { value, path }
}

pub fn save(path: &PathBuf, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create settings directory: {error}"))?;
    }
    let content = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Could not encode settings: {error}"))?;
    fs::write(path, content).map_err(|error| format!("Could not save settings: {error}"))
}
