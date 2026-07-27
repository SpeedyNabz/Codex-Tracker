//! Owns the live application runtime: Codex discovery and supervision,
//! usage refreshes, settings commands, notifications, and frontend events.
//! Made by Heavymask — https://heavymask.com

use crate::{
    model::{
        normalize_snapshot, AppStateDto, CheckpointNotification, ConnectionStatus, UsageSnapshot,
    },
    protocol::{RpcClient, RpcEvent},
    settings::{
        normalize_checkpoint_percentages, normalize_refresh_interval_secs, save as save_settings,
        Settings,
    },
};
use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};
#[cfg(any(windows, unix))]
use std::fs;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tauri::{menu::CheckMenuItem, AppHandle, Emitter, LogicalSize, Manager, State, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;
use tokio::{
    process::Command,
    sync::{Mutex, Notify, RwLock},
    time::{sleep, timeout, MissedTickBehavior},
};

const COMPACT_WIDTH: f64 = 300.0;
const COMPACT_HEIGHT: f64 = 188.0;
const MIN_COMPACT_HEIGHT: f64 = 120.0;
const EXPANDED_HEIGHT: f64 = 520.0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStartDto {
    pub auth_url: String,
}

pub struct AppRuntime {
    app: AppHandle,
    state: RwLock<AppStateDto>,
    settings: StdMutex<Settings>,
    settings_path: PathBuf,
    client: Mutex<Option<Arc<RpcClient>>>,
    refresh_lock: Mutex<()>,
    refresh_interval_secs: AtomicU64,
    refresh_interval_changed: Notify,
    restart: Notify,
    stopping: AtomicBool,
    tray_autostart: StdMutex<Option<CheckMenuItem<Wry>>>,
}

impl AppRuntime {
    pub fn new(
        app: AppHandle,
        settings: Settings,
        settings_path: PathBuf,
        autostart_enabled: bool,
    ) -> Arc<Self> {
        let refresh_interval_secs = normalize_refresh_interval_secs(settings.refresh_interval_secs);
        let mut settings = settings;
        settings.refresh_interval_secs = refresh_interval_secs;
        settings.checkpoint_percentages =
            normalize_checkpoint_percentages(&settings.checkpoint_percentages);
        let state = AppStateDto {
            autostart_enabled,
            expanded: settings.expanded,
            refresh_interval_secs,
            checkpoint_percentages: settings.checkpoint_percentages.clone(),
            ..AppStateDto::default()
        };
        Arc::new(Self {
            app,
            state: RwLock::new(state),
            settings: StdMutex::new(settings),
            settings_path,
            client: Mutex::new(None),
            refresh_lock: Mutex::new(()),
            refresh_interval_secs: AtomicU64::new(refresh_interval_secs),
            refresh_interval_changed: Notify::new(),
            restart: Notify::new(),
            stopping: AtomicBool::new(false),
            tray_autostart: StdMutex::new(None),
        })
    }

    pub fn set_tray_autostart_item(&self, item: CheckMenuItem<Wry>) {
        *self.tray_autostart.lock().expect("tray item lock poisoned") = Some(item);
    }

    pub async fn supervise(self: Arc<Self>) {
        let mut reconnect_attempt = 0usize;
        while !self.stopping.load(Ordering::SeqCst) {
            let executable = match self.resolve_executable().await {
                Some(path) => path,
                None => {
                    self.update_state(|state| {
                        state.status = ConnectionStatus::NeedsCodex;
                        state.message = Some(
                            "Codex CLI not found. Checked PATH and common install locations; choose the Codex executable to continue.".to_string(),
                        );
                        state.codex_path = None;
                        state.codex_version = None;
                        state.updating = false;
                        mark_snapshot_stale(state);
                    })
                    .await;
                    self.restart.notified().await;
                    continue;
                }
            };

            let version = match codex_version(&executable).await {
                Ok(version) => version,
                Err(error) => {
                    self.update_state(|state| {
                        state.status = ConnectionStatus::NeedsCodex;
                        state.message = Some(public_error(&error));
                        state.codex_path = Some(executable.display().to_string());
                        state.codex_version = None;
                        state.updating = false;
                        mark_snapshot_stale(state);
                    })
                    .await;
                    self.restart.notified().await;
                    continue;
                }
            };

            self.update_state(|state| {
                state.status = if state.snapshot.is_some() {
                    ConnectionStatus::Reconnecting
                } else {
                    ConnectionStatus::Starting
                };
                state.message = Some("Connecting to Codex…".to_string());
                state.codex_path = Some(executable.display().to_string());
                state.codex_version = Some(version.clone());
            })
            .await;

            let (client, mut events) = match RpcClient::spawn(&executable).await {
                Ok(value) => value,
                Err(error) => {
                    self.connection_failure(&error).await;
                    self.wait_to_retry(reconnect_attempt).await;
                    reconnect_attempt = reconnect_attempt.saturating_add(1);
                    continue;
                }
            };

            if let Err(error) = client.initialize().await {
                client.shutdown().await;
                self.connection_failure(&format!(
                    "This Codex app-server could not initialize: {error}"
                ))
                .await;
                self.wait_to_retry(reconnect_attempt).await;
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                continue;
            }

            *self.client.lock().await = Some(Arc::clone(&client));
            reconnect_attempt = 0;
            self.refresh_with_client(&client).await;

            let mut interval = self.refresh_interval();
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            interval.tick().await;
            let mut reconnect = true;
            loop {
                let refresh_interval_changed = self.refresh_interval_changed.notified();
                tokio::select! {
                    _ = interval.tick() => self.refresh_with_client(&client).await,
                    _ = refresh_interval_changed => {
                        interval = self.refresh_interval();
                        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                        interval.tick().await;
                    }
                    _ = self.restart.notified() => {
                        reconnect = false;
                        client.shutdown().await;
                        break;
                    }
                    event = events.recv() => match event {
                        Some(RpcEvent::Notification { method, params }) => {
                            self.handle_notification(&client, &method, &params).await;
                        }
                        Some(RpcEvent::Exited) | None => break,
                    }
                }
                if self.stopping.load(Ordering::SeqCst) {
                    reconnect = false;
                    break;
                }
            }

            client.shutdown().await;
            *self.client.lock().await = None;
            if self.stopping.load(Ordering::SeqCst) {
                break;
            }
            if reconnect {
                self.connection_failure("Codex app-server disconnected; reconnecting.")
                    .await;
                self.wait_to_retry(reconnect_attempt).await;
                reconnect_attempt = reconnect_attempt.saturating_add(1);
            }
        }
    }

    async fn handle_notification(&self, client: &Arc<RpcClient>, method: &str, params: &Value) {
        match method {
            "account/rateLimits/updated" => {
                sleep(Duration::from_millis(250)).await;
                self.refresh_with_client(client).await;
            }
            "account/login/completed" => {
                if params.get("success").and_then(Value::as_bool) == Some(true) {
                    self.refresh_with_client(client).await;
                } else {
                    let message = params
                        .get("error")
                        .and_then(Value::as_str)
                        .map(public_error)
                        .unwrap_or_else(|| "Codex sign-in did not complete.".to_string());
                    self.update_state(|state| {
                        state.status = ConnectionStatus::NeedsAuth;
                        state.message = Some(message);
                        state.updating = false;
                        mark_snapshot_stale(state);
                    })
                    .await;
                }
            }
            _ => {}
        }
    }

    async fn refresh_with_client(&self, client: &Arc<RpcClient>) {
        let Ok(_guard) = self.refresh_lock.try_lock() else {
            return;
        };
        self.update_state(|state| state.updating = true).await;

        let account = client
            .request("account/read", Some(json!({ "refreshToken": false })))
            .await;
        match account {
            Ok(account)
                if account.get("account").is_none()
                    || account.get("account").is_some_and(Value::is_null) =>
            {
                self.update_state(|state| {
                    state.status = ConnectionStatus::NeedsAuth;
                    state.message = Some("Sign in with Codex to read your usage.".to_string());
                    state.updating = false;
                    mark_snapshot_stale(state);
                })
                .await;
                return;
            }
            Err(error) if is_auth_error(&error) => {
                self.update_state(|state| {
                    state.status = ConnectionStatus::NeedsAuth;
                    state.message = Some("Your Codex sign-in needs to be renewed.".to_string());
                    state.updating = false;
                    mark_snapshot_stale(state);
                })
                .await;
                return;
            }
            Err(error) => {
                self.connection_failure(&error).await;
                return;
            }
            Ok(_) => {}
        }

        let (limits, usage) = tokio::join!(
            client.request("account/rateLimits/read", None),
            client.request("account/usage/read", None)
        );
        let limits = match limits {
            Ok(limits) => limits,
            Err(error) if is_auth_error(&error) => {
                self.update_state(|state| {
                    state.status = ConnectionStatus::NeedsAuth;
                    state.message = Some("Sign in with Codex to read your usage.".to_string());
                    state.updating = false;
                    mark_snapshot_stale(state);
                })
                .await;
                return;
            }
            Err(error) => {
                self.connection_failure(&error).await;
                return;
            }
        };

        let usage_warning = usage.as_ref().err().map(|_| {
            "Allowance is current, but token history is temporarily unavailable.".to_string()
        });
        let today = Local::now().format("%Y-%m-%d").to_string();
        let updated_at = chrono::Utc::now().timestamp();
        match normalize_snapshot(&limits, usage.as_ref().ok(), &today, updated_at) {
            Ok(snapshot) => {
                let checkpoint_notification = {
                    let checkpoints = self
                        .settings
                        .lock()
                        .expect("settings lock poisoned")
                        .checkpoint_percentages
                        .clone();
                    let previous = self.state.read().await;
                    checkpoint_message(previous.snapshot.as_ref(), &snapshot, &checkpoints).map(
                        |message| CheckpointNotification {
                            id: format!("checkpoint-{updated_at}"),
                            message,
                        },
                    )
                };
                self.update_state(|state| {
                    state.status = ConnectionStatus::Ready;
                    state.snapshot = Some(snapshot);
                    state.message = usage_warning;
                    state.updating = false;
                    if let Some(notification) = checkpoint_notification.as_ref() {
                        state.checkpoint_notification = Some(notification.clone());
                    }
                })
                .await;
                if let Some(notification) = checkpoint_notification {
                    self.send_checkpoint_notification(&notification.message);
                }
            }
            Err(error) => {
                self.update_state(|state| {
                    state.status = ConnectionStatus::Error;
                    state.message = Some(public_error(&format!(
                        "Codex returned an unsupported usage response: {error}"
                    )));
                    state.updating = false;
                    mark_snapshot_stale(state);
                })
                .await;
            }
        }
    }

    pub async fn refresh(&self) -> Result<(), String> {
        let client = self
            .client
            .lock()
            .await
            .clone()
            .ok_or_else(|| "Codex is not connected yet".to_string())?;
        self.refresh_with_client(&client).await;
        Ok(())
    }

    pub async fn begin_login(&self) -> Result<LoginStartDto, String> {
        let client = self
            .client
            .lock()
            .await
            .clone()
            .ok_or_else(|| "Codex is not connected yet".to_string())?;
        let result = client
            .request(
                "account/login/start",
                Some(json!({
                    "type": "chatgpt",
                    "codexStreamlinedLogin": true,
                    "useHostedLoginSuccessPage": true
                })),
            )
            .await?;
        let auth_url = result
            .get("authUrl")
            .and_then(Value::as_str)
            .filter(|url| url.starts_with("https://"))
            .ok_or_else(|| "Codex did not return a safe sign-in URL".to_string())?
            .to_string();
        self.update_state(|state| {
            state.status = ConnectionStatus::NeedsAuth;
            state.message = Some("Complete sign-in in your browser.".to_string());
        })
        .await;
        Ok(LoginStartDto { auth_url })
    }

    pub async fn set_codex_path(&self, path: Option<String>) -> Result<(), String> {
        if let Some(path) = path.as_deref() {
            let path = Path::new(path);
            if !path.is_file() {
                return Err("Choose the installed Codex executable.".to_string());
            }
            #[cfg(windows)]
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .map_or(true, |extension| !extension.eq_ignore_ascii_case("exe"))
            {
                return Err("Choose the installed Codex executable.".to_string());
            }
            codex_version(path).await?;
        }
        {
            let mut settings = self.settings.lock().expect("settings lock poisoned");
            settings.codex_executable = path;
            save_settings(&self.settings_path, &settings)?;
        }
        self.update_state(|state| {
            state.status = ConnectionStatus::Starting;
            state.message = Some("Restarting the Codex connection…".to_string());
        })
        .await;
        self.restart.notify_one();
        Ok(())
    }

    pub async fn set_autostart(&self, enabled: bool) -> Result<(), String> {
        let manager = self.app.autolaunch();
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(|error| format!("Could not update startup: {error}"))?;
        if let Some(item) = self
            .tray_autostart
            .lock()
            .expect("tray item lock poisoned")
            .as_ref()
        {
            let _ = item.set_checked(enabled);
        }
        self.update_state(|state| state.autostart_enabled = enabled)
            .await;
        Ok(())
    }

    pub async fn set_expanded(&self, expanded: bool) -> Result<(), String> {
        let window = self
            .app
            .get_webview_window("main")
            .ok_or_else(|| "Overlay window is unavailable".to_string())?;
        let height = if expanded {
            EXPANDED_HEIGHT
        } else {
            COMPACT_HEIGHT
        };
        window
            .set_size(LogicalSize::new(COMPACT_WIDTH, height))
            .map_err(|error| format!("Could not resize overlay: {error}"))?;
        {
            let mut settings = self.settings.lock().expect("settings lock poisoned");
            settings.expanded = expanded;
            save_settings(&self.settings_path, &settings)?;
        }
        self.update_state(|state| state.expanded = expanded).await;
        Ok(())
    }

    pub async fn set_refresh_interval(&self, seconds: u64) -> Result<(), String> {
        let seconds = normalize_refresh_interval_secs(seconds);
        {
            let mut settings = self.settings.lock().expect("settings lock poisoned");
            settings.refresh_interval_secs = seconds;
            save_settings(&self.settings_path, &settings)?;
        }
        self.refresh_interval_secs.store(seconds, Ordering::Relaxed);
        self.update_state(|state| state.refresh_interval_secs = seconds)
            .await;
        self.refresh_interval_changed.notify_one();
        Ok(())
    }

    pub async fn set_checkpoint_percentages(&self, percentages: Vec<u8>) -> Result<(), String> {
        let percentages = normalize_checkpoint_percentages(&percentages);
        {
            let mut settings = self.settings.lock().expect("settings lock poisoned");
            settings.checkpoint_percentages = percentages.clone();
            save_settings(&self.settings_path, &settings)?;
        }
        self.update_state(|state| state.checkpoint_percentages = percentages)
            .await;
        Ok(())
    }

    pub async fn dismiss_checkpoint_notification(&self) -> Result<(), String> {
        self.update_state(|state| state.checkpoint_notification = None)
            .await;
        Ok(())
    }

    pub async fn set_overlay_height(&self, height: f64) -> Result<(), String> {
        if !height.is_finite() {
            return Err("Overlay height must be finite".to_string());
        }
        let window = self
            .app
            .get_webview_window("main")
            .ok_or_else(|| "Overlay window is unavailable".to_string())?;
        let height = height.clamp(MIN_COMPACT_HEIGHT, EXPANDED_HEIGHT);
        window
            .set_size(LogicalSize::new(COMPACT_WIDTH, height))
            .map_err(|error| format!("Could not resize overlay: {error}"))
    }

    pub async fn record_window_position(&self, x: i32, y: i32) {
        let mut settings = self.settings.lock().expect("settings lock poisoned");
        settings.window_x = Some(x);
        settings.window_y = Some(y);
        let _ = save_settings(&self.settings_path, &settings);
    }

    pub async fn state(&self) -> AppStateDto {
        self.state.read().await.clone()
    }

    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.restart.notify_waiters();
        if let Some(client) = self.client.lock().await.clone() {
            client.shutdown().await;
        }
    }

    async fn resolve_executable(&self) -> Option<PathBuf> {
        let configured = self
            .settings
            .lock()
            .expect("settings lock poisoned")
            .codex_executable
            .clone();

        let mut candidates = Vec::new();
        if let Some(path) = configured {
            push_unique_path(&mut candidates, PathBuf::from(path));
        }

        #[cfg(windows)]
        candidates.extend(windows_codex_candidates().await);

        #[cfg(unix)]
        candidates.extend(unix_codex_candidates());

        #[cfg(not(any(windows, unix)))]
        push_unique_path(&mut candidates, PathBuf::from("codex"));

        for candidate in candidates {
            if (candidate.is_file() || candidate == Path::new("codex"))
                && codex_version(&candidate).await.is_ok()
            {
                return Some(candidate);
            }
        }

        None
    }

    async fn connection_failure(&self, error: &str) {
        self.update_state(|state| {
            state.status = ConnectionStatus::Reconnecting;
            state.message = Some(public_error(error));
            state.updating = false;
            mark_snapshot_stale(state);
        })
        .await;
    }

    async fn wait_to_retry(&self, attempt: usize) {
        let delay = backoff_for(attempt);
        tokio::select! {
            _ = sleep(delay) => {},
            _ = self.restart.notified() => {},
        }
    }

    async fn update_state(&self, update: impl FnOnce(&mut AppStateDto)) {
        let snapshot = {
            let mut state = self.state.write().await;
            update(&mut state);
            state.clone()
        };
        let _ = self.app.emit("usage-state-changed", snapshot);
    }

    fn send_checkpoint_notification(&self, message: &str) {
        if let Err(error) = self
            .app
            .notification()
            .builder()
            .title("Codex Tracker")
            .body(message)
            .show()
        {
            eprintln!("Could not show checkpoint notification: {error}");
        }
    }

    fn refresh_interval(&self) -> tokio::time::Interval {
        let seconds = self.refresh_interval_secs.load(Ordering::Relaxed).max(1);
        tokio::time::interval(Duration::from_secs(seconds))
    }
}

fn checkpoint_message(
    previous: Option<&UsageSnapshot>,
    current: &UsageSnapshot,
    checkpoints: &[u8],
) -> Option<String> {
    let previous = previous?;
    let mut reached = Vec::new();

    for group in &current.quota_groups {
        let Some(previous_group) = previous
            .quota_groups
            .iter()
            .find(|candidate| candidate.id == group.id)
        else {
            continue;
        };

        for window in &group.windows {
            let Some(previous_window) = previous_group
                .windows
                .iter()
                .find(|candidate| candidate.key == window.key)
            else {
                continue;
            };

            let crossed = checkpoints.iter().copied().filter(|checkpoint| {
                previous_window.remaining_percent > f64::from(*checkpoint)
                    && window.remaining_percent <= f64::from(*checkpoint)
            });
            for checkpoint in crossed {
                reached.push(format!(
                    "{} reached {}% remaining.",
                    window.label, checkpoint
                ));
            }
        }
    }

    (!reached.is_empty()).then(|| format!("Checkpoint reached: {}", reached.join(" ")))
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    let normalized = path.to_string_lossy();
    #[cfg(windows)]
    let normalized = normalized.to_ascii_lowercase();
    #[cfg(not(windows))]
    let normalized = normalized.into_owned();
    if !paths.iter().any(|existing| {
        let existing = existing.to_string_lossy();
        #[cfg(windows)]
        let existing = existing.to_ascii_lowercase();
        #[cfg(not(windows))]
        let existing = existing.into_owned();
        existing == normalized
    }) {
        paths.push(path);
    }
}

#[cfg(unix)]
fn unix_codex_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            push_unique_path(&mut candidates, directory.join("codex"));
        }
    }

    for path in [
        "/usr/local/bin/codex",
        "/usr/bin/codex",
        "/opt/homebrew/bin/codex",
        "/opt/local/bin/codex",
    ] {
        push_unique_path(&mut candidates, PathBuf::from(path));
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for path in [
            home.join(".local/bin/codex"),
            home.join(".cargo/bin/codex"),
            home.join(".npm-global/bin/codex"),
        ] {
            push_unique_path(&mut candidates, path);
        }

        for root in [
            home.join(".vscode/extensions"),
            home.join(".vscode-insiders/extensions"),
            home.join(".cursor/extensions"),
            home.join(".nvm/versions/node"),
        ] {
            for path in find_codex_executables(&root, 4) {
                push_unique_path(&mut candidates, path);
            }
        }
    }

    candidates
}

#[cfg(windows)]
async fn windows_codex_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // This is the cheapest and most intentional lookup, but an autostarted app
    // can inherit a PATH that predates a Codex or VS Code installation.
    if let Ok(output) = Command::new("where.exe")
        .arg("codex.exe")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
    {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let path = line.trim().trim_matches('"');
                if !path.is_empty() {
                    push_unique_path(&mut candidates, PathBuf::from(path));
                }
            }
        }
    }

    // Also inspect PATH entries directly so a non-standard `where.exe` result
    // or a quoted PATH entry cannot hide the executable.
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            push_unique_path(&mut candidates, directory.join("codex.exe"));
        }
    }

    for root in windows_codex_search_roots() {
        for path in find_codex_executables(&root, 4) {
            push_unique_path(&mut candidates, path);
        }
    }

    candidates
}

#[cfg(windows)]
fn windows_codex_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let profile = PathBuf::from(profile);
        roots.extend([
            profile.join(".vscode\\extensions"),
            profile.join(".vscode-insiders\\extensions"),
            profile.join(".cursor\\extensions"),
            profile.join(".local\\bin"),
        ]);
    }

    if let Some(app_data) = std::env::var_os("APPDATA") {
        let app_data = PathBuf::from(app_data);
        roots.extend([
            app_data.join("npm"),
            app_data.join("Code\\User\\extensions"),
            app_data.join("Code - Insiders\\User\\extensions"),
            app_data.join("VSCodium\\User\\extensions"),
        ]);
    }

    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let local_app_data = PathBuf::from(local_app_data);
        roots.extend([
            local_app_data.join("Programs\\Codex"),
            local_app_data.join("Programs\\OpenAI"),
            local_app_data.join("OpenAI"),
            local_app_data.join("Microsoft\\WinGet\\Packages"),
        ]);
    }

    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        let program_files = PathBuf::from(program_files);
        roots.extend([program_files.join("Codex"), program_files.join("OpenAI")]);
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            roots.push(directory.to_path_buf());
        }
    }

    roots
}

fn find_codex_executables(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    fn visit(directory: &Path, depth: usize, max_depth: usize, results: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if path.is_file() && is_codex_executable(&path) {
                results.push(path);
            } else if file_type.is_dir() && depth < max_depth {
                visit(&path, depth + 1, max_depth, results);
            }
        }
    }

    let mut results = Vec::new();
    if root.is_dir() {
        visit(root, 0, max_depth, &mut results);
    } else if is_codex_executable(root) {
        results.push(root.to_path_buf());
    }
    results
}

fn is_codex_executable(path: &Path) -> bool {
    let expected_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    path.is_file()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                if cfg!(windows) {
                    name.eq_ignore_ascii_case(expected_name)
                } else {
                    name == expected_name
                }
            })
}

fn mark_snapshot_stale(state: &mut AppStateDto) {
    if let Some(snapshot) = state.snapshot.as_mut() {
        snapshot.stale = true;
    }
}

fn is_auth_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("authentication required")
        || error.contains("unauthorized")
        || error.contains("not logged in")
        || error.contains("sign in")
}

fn public_error(error: &str) -> String {
    let without_urls = error
        .split_whitespace()
        .map(|part| {
            if part.starts_with("http://") || part.starts_with("https://") {
                "[link hidden]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    without_urls.chars().take(240).collect()
}

fn backoff_for(attempt: usize) -> Duration {
    const SECONDS: [u64; 6] = [1, 2, 5, 15, 30, 60];
    Duration::from_secs(SECONDS[attempt.min(SECONDS.len() - 1)])
}

async fn codex_version(executable: &Path) -> Result<String, String> {
    let mut process = std::process::Command::new(executable);
    process
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }
    let output = timeout(Duration::from_secs(10), Command::from(process).output())
        .await
        .map_err(|_| "Timed out while checking the Codex executable".to_string())?
        .map_err(|error| format!("Could not run the Codex executable: {error}"))?;
    if !output.status.success() {
        return Err("The selected file is not a working Codex CLI executable.".to_string());
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !version.to_ascii_lowercase().contains("codex") {
        return Err("The selected executable did not identify itself as Codex.".to_string());
    }
    Ok(version)
}

#[tauri::command]
pub async fn get_app_state(runtime: State<'_, Arc<AppRuntime>>) -> Result<AppStateDto, String> {
    Ok(runtime.state().await)
}

#[tauri::command]
pub async fn refresh_usage(runtime: State<'_, Arc<AppRuntime>>) -> Result<(), String> {
    runtime.refresh().await
}

#[tauri::command]
pub async fn begin_chatgpt_login(
    runtime: State<'_, Arc<AppRuntime>>,
) -> Result<LoginStartDto, String> {
    runtime.begin_login().await
}

#[tauri::command]
pub async fn set_codex_executable(
    runtime: State<'_, Arc<AppRuntime>>,
    path: Option<String>,
) -> Result<(), String> {
    runtime.set_codex_path(path).await
}

#[tauri::command]
pub async fn set_autostart_enabled(
    runtime: State<'_, Arc<AppRuntime>>,
    enabled: bool,
) -> Result<(), String> {
    runtime.set_autostart(enabled).await
}

#[tauri::command]
pub async fn set_overlay_expanded(
    runtime: State<'_, Arc<AppRuntime>>,
    expanded: bool,
) -> Result<(), String> {
    runtime.set_expanded(expanded).await
}

#[tauri::command]
pub async fn set_refresh_interval(
    runtime: State<'_, Arc<AppRuntime>>,
    seconds: u64,
) -> Result<(), String> {
    runtime.set_refresh_interval(seconds).await
}

#[tauri::command]
pub async fn set_checkpoint_percentages(
    runtime: State<'_, Arc<AppRuntime>>,
    percentages: Vec<u8>,
) -> Result<(), String> {
    runtime.set_checkpoint_percentages(percentages).await
}

#[tauri::command]
pub async fn dismiss_checkpoint_notification(
    runtime: State<'_, Arc<AppRuntime>>,
) -> Result<(), String> {
    runtime.dismiss_checkpoint_notification().await
}

#[tauri::command]
pub async fn set_overlay_height(
    runtime: State<'_, Arc<AppRuntime>>,
    height: f64,
) -> Result<(), String> {
    runtime.set_overlay_height(height).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{QuotaGroup, QuotaWindow, TokenActivity};

    #[test]
    fn backoff_is_bounded() {
        assert_eq!(backoff_for(0), Duration::from_secs(1));
        assert_eq!(backoff_for(3), Duration::from_secs(15));
        assert_eq!(backoff_for(100), Duration::from_secs(60));
    }

    #[test]
    fn auth_errors_are_classified() {
        assert!(is_auth_error("codex account authentication required"));
        assert!(is_auth_error("Unauthorized"));
        assert!(!is_auth_error("network timed out"));
    }

    #[test]
    fn public_errors_hide_urls_and_bound_length() {
        let error = format!("open https://example.test/secret {}", "x".repeat(300));
        let public = public_error(&error);
        assert!(!public.contains("example.test"));
        assert!(public.chars().count() <= 240);
    }

    #[cfg(windows)]
    #[test]
    fn unique_paths_are_case_insensitive() {
        let mut paths = Vec::new();
        push_unique_path(&mut paths, PathBuf::from(r"C:\Users\Test\codex.exe"));
        push_unique_path(&mut paths, PathBuf::from(r"c:\users\test\CODEX.EXE"));
        assert_eq!(paths.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn unique_paths_preserve_case_on_unix() {
        let mut paths = Vec::new();
        push_unique_path(&mut paths, PathBuf::from("/opt/Codex/codex"));
        push_unique_path(&mut paths, PathBuf::from("/opt/codex/codex"));
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn refresh_interval_is_bounded() {
        assert_eq!(normalize_refresh_interval_secs(0), 15);
        assert_eq!(normalize_refresh_interval_secs(60), 60);
        assert_eq!(normalize_refresh_interval_secs(600), 300);
    }

    fn snapshot_with_remaining(remaining_percent: f64) -> UsageSnapshot {
        UsageSnapshot {
            quota_groups: vec![QuotaGroup {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                primary: true,
                plan_type: None,
                windows: vec![QuotaWindow {
                    key: "primary".to_string(),
                    label: "5-hour allowance".to_string(),
                    used_percent: 100.0 - remaining_percent,
                    remaining_percent,
                    window_duration_mins: Some(300),
                    resets_at: None,
                }],
            }],
            token_activity: TokenActivity::default(),
            credits: None,
            updated_at: 1,
            stale: false,
        }
    }

    #[test]
    fn checkpoint_message_only_reports_downward_crossings() {
        let previous = snapshot_with_remaining(72.0);
        let current = snapshot_with_remaining(49.0);
        let message = checkpoint_message(Some(&previous), &current, &[50, 20]);
        assert_eq!(
            message.as_deref(),
            Some("Checkpoint reached: 5-hour allowance reached 50% remaining.")
        );

        let still_below = snapshot_with_remaining(48.0);
        assert!(checkpoint_message(Some(&current), &still_below, &[50, 20]).is_none());
    }

    #[test]
    fn checkpoint_message_combines_multiple_levels_crossed_in_one_refresh() {
        let previous = snapshot_with_remaining(25.0);
        let current = snapshot_with_remaining(9.0);
        assert_eq!(
            checkpoint_message(Some(&previous), &current, &[50, 20, 10]).as_deref(),
            Some(
                "Checkpoint reached: 5-hour allowance reached 20% remaining. 5-hour allowance reached 10% remaining."
            )
        );
    }

    #[cfg(windows)]
    #[test]
    fn finds_codex_inside_an_extension_install() {
        let root = tempfile::tempdir().expect("temporary directory");
        let executable = root
            .path()
            .join("openai.chatgpt-26.715.31925-win32-x64")
            .join("bin")
            .join("windows-x86_64")
            .join("codex.exe");
        std::fs::create_dir_all(executable.parent().expect("executable parent"))
            .expect("extension directories");
        std::fs::File::create(&executable).expect("codex executable");

        let found = find_codex_executables(root.path(), 4);
        assert_eq!(found, vec![executable]);
    }
}
