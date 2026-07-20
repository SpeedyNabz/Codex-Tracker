use crate::{
    model::{normalize_snapshot, AppStateDto, ConnectionStatus},
    protocol::{RpcClient, RpcEvent},
    settings::{save as save_settings, Settings},
};
use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};
#[cfg(windows)]
use std::fs;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tauri::{menu::CheckMenuItem, AppHandle, Emitter, LogicalSize, Manager, State, Wry};
use tauri_plugin_autostart::ManagerExt;
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
        let state = AppStateDto {
            autostart_enabled,
            expanded: settings.expanded,
            ..AppStateDto::default()
        };
        Arc::new(Self {
            app,
            state: RwLock::new(state),
            settings: StdMutex::new(settings),
            settings_path,
            client: Mutex::new(None),
            refresh_lock: Mutex::new(()),
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
                            "Codex CLI not found. Checked PATH and common install locations; choose codex.exe to continue.".to_string(),
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

            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            interval.tick().await;
            let mut reconnect = true;
            loop {
                tokio::select! {
                    _ = interval.tick() => self.refresh_with_client(&client).await,
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
                self.update_state(|state| {
                    state.status = ConnectionStatus::Ready;
                    state.snapshot = Some(snapshot);
                    state.message = usage_warning;
                    state.updating = false;
                })
                .await;
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
            if !path.is_file()
                || path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map_or(true, |extension| !extension.eq_ignore_ascii_case("exe"))
            {
                return Err("Choose the installed codex.exe file.".to_string());
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
        .map_err(|error| format!("Could not update Windows startup: {error}"))?;
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
        {
            candidates.extend(windows_codex_candidates().await);

            for candidate in candidates {
                if candidate.is_file() && codex_version(&candidate).await.is_ok() {
                    return Some(candidate);
                }
            }

            None
        }

        #[cfg(not(windows))]
        {
            Some(PathBuf::from("codex"))
        }
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
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    let normalized = path.to_string_lossy().to_ascii_lowercase();
    if !paths
        .iter()
        .any(|existing| existing.to_string_lossy().to_ascii_lowercase() == normalized)
    {
        paths.push(path);
    }
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
        roots.extend([
            program_files.join("Codex"),
            program_files.join("OpenAI"),
        ]);
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            roots.push(directory.to_path_buf());
        }
    }

    roots
}

#[cfg(windows)]
fn find_codex_executables(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    fn visit(
        directory: &Path,
        depth: usize,
        max_depth: usize,
        results: &mut Vec<PathBuf>,
    ) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
            {
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

#[cfg(windows)]
fn is_codex_executable(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
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
        .map_err(|_| "Timed out while checking codex.exe".to_string())?
        .map_err(|error| format!("Could not run codex.exe: {error}"))?;
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
pub async fn set_overlay_height(
    runtime: State<'_, Arc<AppRuntime>>,
    height: f64,
) -> Result<(), String> {
    runtime.set_overlay_height(height).await
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn unique_paths_are_case_insensitive() {
        let mut paths = Vec::new();
        push_unique_path(&mut paths, PathBuf::from(r"C:\Users\Test\codex.exe"));
        push_unique_path(&mut paths, PathBuf::from(r"c:\users\test\CODEX.EXE"));
        assert_eq!(paths.len(), 1);
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
