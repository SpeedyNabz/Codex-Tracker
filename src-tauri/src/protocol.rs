use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, oneshot, Mutex},
    time::{sleep, timeout},
};

type PendingResult = Result<Value, String>;
type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<PendingResult>>>>;

#[derive(Debug)]
pub enum RpcEvent {
    Notification { method: String, params: Value },
    Exited,
}

pub struct RpcClient {
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Option<Child>>>,
    pending: PendingMap,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

impl RpcClient {
    pub async fn spawn(
        executable: &Path,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<RpcEvent>), String> {
        let mut process = std::process::Command::new(executable);
        process
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            process.creation_flags(CREATE_NO_WINDOW);
        }

        let mut command = Command::from(process);
        command.kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start Codex: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Codex app-server did not expose stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Codex app-server did not expose stdout".to_string())?;
        let stderr = child.stderr.take();

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let child = Arc::new(Mutex::new(Some(child)));
        let closed = Arc::new(AtomicBool::new(false));
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let client = Arc::new(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            child: Arc::clone(&child),
            pending: Arc::clone(&pending),
            next_id: AtomicU64::new(1),
            closed: Arc::clone(&closed),
        });

        let reader_pending = Arc::clone(&pending);
        let reader_closed = Arc::clone(&closed);
        let reader_events = event_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if handle_line(&line, &reader_pending, &reader_events)
                    .await
                    .is_err()
                {
                    // Malformed and unknown lines are deliberately ignored. A later valid
                    // response or the request timeout gives the caller a safe recovery path.
                }
            }
            if !reader_closed.swap(true, Ordering::SeqCst) {
                fail_pending(&reader_pending, "Codex app-server disconnected").await;
                let _ = reader_events.send(RpcEvent::Exited);
            }
        });

        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while matches!(lines.next_line().await, Ok(Some(_))) {
                    // Drain diagnostics so the child cannot block. Do not log raw app-server
                    // output because it may contain account or login details.
                }
            });
        }

        let monitor_child = Arc::clone(&child);
        let monitor_closed = Arc::clone(&closed);
        let monitor_pending = Arc::clone(&pending);
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(1)).await;
                let exited = {
                    let mut child = monitor_child.lock().await;
                    match child.as_mut() {
                        Some(child) => matches!(child.try_wait(), Ok(Some(_))),
                        None => true,
                    }
                };
                if exited {
                    if !monitor_closed.swap(true, Ordering::SeqCst) {
                        fail_pending(&monitor_pending, "Codex app-server exited").await;
                        let _ = event_tx.send(RpcEvent::Exited);
                    }
                    break;
                }
            }
        });

        Ok((client, event_rx))
    }

    pub async fn initialize(&self) -> Result<Value, String> {
        let response = self
            .request(
                "initialize",
                Some(json!({
                    "clientInfo": {
                        "name": "codex_usage_overlay",
                        "title": "Codex Tracker",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "experimentalApi": false,
                        "requestAttestation": false
                    }
                })),
            )
            .await?;
        self.notify("initialized", None).await?;
        Ok(response)
    }

    pub async fn request(&self, method: &str, params: Option<Value>) -> PendingResult {
        if self.closed.load(Ordering::SeqCst) {
            return Err("Codex app-server is not connected".to_string());
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let message = match params {
            Some(params) => json!({ "method": method, "id": id, "params": params }),
            None => json!({ "method": method, "id": id }),
        };
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);

        if let Err(error) = self.write_message(&message).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }

        match timeout(Duration::from_secs(15), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(format!("Codex request {method} was cancelled")),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("Codex request {method} timed out"))
            }
        }
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), String> {
        let message = match params {
            Some(params) => json!({ "method": method, "params": params }),
            None => json!({ "method": method }),
        };
        self.write_message(&message).await
    }

    async fn write_message(&self, message: &Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(message)
            .map_err(|error| format!("Could not encode Codex request: {error}"))?;
        bytes.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| format!("Could not write to Codex app-server: {error}"))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("Could not flush Codex request: {error}"))
    }

    pub async fn shutdown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        fail_pending(&self.pending, "Codex app-server stopped").await;
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

async fn handle_line(
    line: &str,
    pending: &PendingMap,
    events: &mpsc::UnboundedSender<RpcEvent>,
) -> Result<(), String> {
    let message: Value = serde_json::from_str(line)
        .map_err(|error| format!("Invalid Codex JSON-RPC message: {error}"))?;

    if let Some(id) = message.get("id").and_then(Value::as_u64) {
        if let Some(sender) = pending.lock().await.remove(&id) {
            let result = if let Some(error) = message.get("error") {
                Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex request failed")
                    .to_string())
            } else {
                Ok(message.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = sender.send(result);
        }
        return Ok(());
    }

    if let Some(method) = message.get("method").and_then(Value::as_str) {
        let _ = events.send(RpcEvent::Notification {
            method: method.to_string(),
            params: message.get("params").cloned().unwrap_or(Value::Null),
        });
    }
    Ok(())
}

async fn fail_pending(pending: &PendingMap, message: &str) {
    let requests = {
        let mut pending = pending.lock().await;
        pending
            .drain()
            .map(|(_, sender)| sender)
            .collect::<Vec<_>>()
    };
    for sender in requests {
        let _ = sender.send(Err(message.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn routes_successful_response_to_matching_request() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (sender, receiver) = oneshot::channel();
        pending.lock().await.insert(7, sender);
        let (event_tx, _) = mpsc::unbounded_channel();

        handle_line(r#"{"id":7,"result":{"ok":true}}"#, &pending, &event_tx)
            .await
            .unwrap();
        assert_eq!(receiver.await.unwrap().unwrap(), json!({ "ok": true }));
    }

    #[tokio::test]
    async fn routes_rpc_error_without_exposing_other_fields() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (sender, receiver) = oneshot::channel();
        pending.lock().await.insert(8, sender);
        let (event_tx, _) = mpsc::unbounded_channel();

        handle_line(
            r#"{"id":8,"error":{"code":-32600,"message":"authentication required","data":{"secret":"ignored"}}}"#,
            &pending,
            &event_tx,
        )
        .await
        .unwrap();
        assert_eq!(
            receiver.await.unwrap().unwrap_err(),
            "authentication required"
        );
    }

    #[tokio::test]
    async fn forwards_notifications_and_rejects_malformed_json() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        handle_line(
            r#"{"method":"account/rateLimits/updated","params":{"rateLimits":{}}}"#,
            &pending,
            &event_tx,
        )
        .await
        .unwrap();
        assert!(handle_line("not-json", &pending, &event_tx).await.is_err());

        match event_rx.recv().await.unwrap() {
            RpcEvent::Notification { method, .. } => {
                assert_eq!(method, "account/rateLimits/updated")
            }
            RpcEvent::Exited => panic!("unexpected exit"),
        }
    }
}
