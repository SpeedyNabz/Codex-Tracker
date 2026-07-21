use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};

pub const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 60;
pub const MIN_REFRESH_INTERVAL_SECS: u64 = 15;
pub const MAX_REFRESH_INTERVAL_SECS: u64 = 300;
pub const DEFAULT_CHECKPOINT_PERCENTAGES: &[u8] = &[50, 20, 10];
pub const MIN_CHECKPOINT_PERCENT: u8 = 1;
pub const MAX_CHECKPOINT_PERCENT: u8 = 99;

pub fn normalize_refresh_interval_secs(value: u64) -> u64 {
    value.clamp(MIN_REFRESH_INTERVAL_SECS, MAX_REFRESH_INTERVAL_SECS)
}

pub fn normalize_checkpoint_percentages(values: &[u8]) -> Vec<u8> {
    let mut normalized = values
        .iter()
        .copied()
        .filter(|value| (MIN_CHECKPOINT_PERCENT..=MAX_CHECKPOINT_PERCENT).contains(value))
        .collect::<Vec<_>>();
    normalized.sort_unstable_by(|left, right| right.cmp(left));
    normalized.dedup();
    normalized
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub codex_executable: Option<String>,
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub expanded: bool,
    pub autostart_initialized: bool,
    pub refresh_interval_secs: u64,
    pub checkpoint_percentages: Vec<u8>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            codex_executable: None,
            window_x: None,
            window_y: None,
            expanded: false,
            autostart_initialized: false,
            refresh_interval_secs: DEFAULT_REFRESH_INTERVAL_SECS,
            checkpoint_percentages: DEFAULT_CHECKPOINT_PERCENTAGES.to_vec(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_percentages_are_sorted_deduplicated_and_bounded() {
        assert_eq!(
            normalize_checkpoint_percentages(&[0, 50, 20, 50, 100, 10]),
            vec![50, 20, 10]
        );
    }
}
