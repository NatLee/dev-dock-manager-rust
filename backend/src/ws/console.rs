//! 終端機 WebSocket：先連線（query 僅 ?container=ID），第一則訊息須帶 token 驗證後才處理。
//! 對應 Django ConsoleConsumer：建立 exec 或 attach 取得 Docker 串流，轉發到 WebSocket；支援 PTY 輸入與 resize。

use axum::{
    extract::{Query, State},
    extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    response::{IntoResponse, Response},
};
use axum::http::StatusCode;
use std::sync::atomic::{AtomicBool, Ordering};
use bollard::models::ContainerStateStatusEnum;
use bollard::query_parameters::AttachContainerOptionsBuilder;
use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions};
use futures_util::{SinkExt, StreamExt};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::db::get_by_id;
use crate::jwt;
use crate::AppState;

/// Query 參數：?container=CONTAINER_ID（token 改由第一則訊息傳送，避免進 URL/log）
#[derive(serde::Deserialize)]
pub struct ConsoleQuery {
    pub container: Option<String>,
}

/// 先接受連線，token 於第一則訊息內驗證（見 handle_socket）。
pub async fn handler(
    State(state): State<AppState>,
    Query(params): Query<ConsoleQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let container_id = params.container.as_deref().unwrap_or("").to_string();
    if container_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing container query").into_response();
    }
    let state = Arc::new(state);
    upgrade.on_upgrade(move |socket| handle_socket(socket, state, container_id))
}

/// Session state for one console connection.
struct Session {
    container_id: String,
    exec_id: Option<String>,
    pid_path: Option<String>,
    /// Write half for exec stdin (shell) or attach stdin.
    stdin_tx: Option<Arc<Mutex<Pin<Box<dyn tokio::io::AsyncWrite + Send>>>>>,
}

fn close_unauthorized() -> Option<CloseFrame<'static>> {
    use axum::extract::ws::close_code;
    Some(CloseFrame {
        code: close_code::POLICY,
        reason: std::borrow::Cow::Borrowed("Unauthorized"),
    })
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>, _container_id: String) {
    let (ws_tx, mut ws_rx) = socket.split();
    let session: Arc<Mutex<Option<Session>>> = Arc::new(Mutex::new(None));
    let authenticated = Arc::new(AtomicBool::new(false));

    let session_clone = session.clone();
    let state_clone = state.clone();
    let auth_clone = authenticated.clone();

    let ws_tx = Arc::new(Mutex::new(ws_tx));
    let ws_tx_recv = ws_tx.clone();

    let recv_task = tokio::spawn(async move {
        while let Some(msg) = ws_rx.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let parsed = match serde_json::from_str::<serde_json::Value>(&text) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    let is_first = !auth_clone.load(Ordering::Relaxed);
                    if is_first {
                        let token = parsed.get("token").and_then(|t| t.as_str()).map(str::to_string);
                        let token = match token {
                            Some(t) if !t.is_empty() => t,
                            _ => {
                                let _ = ws_tx_recv.lock().await.send(Message::Close(close_unauthorized())).await;
                                break;
                            }
                        };
                        let claims = match jwt::verify_access(&token, state_clone.config.jwt_secret.as_bytes()) {
                            Ok(c) => c,
                            Err(_) => {
                                let _ = ws_tx_recv.lock().await.send(Message::Close(close_unauthorized())).await;
                                break;
                            }
                        };
                        if get_by_id(&state_clone.pool, claims.user_id).await.ok().flatten().is_none() {
                            let _ = ws_tx_recv.lock().await.send(Message::Close(close_unauthorized())).await;
                            break;
                        }
                        auth_clone.store(true, Ordering::Relaxed);
                    }
                    let action = parsed.get("action").and_then(|a| a.as_str());
                    let payload = parsed.get("payload").cloned().unwrap_or_default();
                    if let Err(e) = handle_message(
                        &state_clone,
                        &session_clone,
                        action,
                        payload,
                        &ws_tx_recv,
                    )
                    .await
                    {
                        tracing::warn!("console message error: {}", e);
                        break;
                    }
                }
                Ok(Message::Close(_)) => break,
                _ => {}
            }
        }
    });

    recv_task.await.ok();

    let mut guard = session.lock().await;
    if let Some(s) = guard.take() {
        cleanup_shell(&state.docker, &s).await;
    }
}

async fn handle_message(
    state: &AppState,
    session: &Arc<Mutex<Option<Session>>>,
    action: Option<&str>,
    payload: serde_json::Value,
    ws_tx: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
) -> Result<(), String> {
    match action {
        Some("shell") => {
            let id = payload
                .get("Id")
                .and_then(|v| v.as_str())
                .ok_or("shell: missing Id")?;
            start_shell(state, session, id, ws_tx).await
        }
        Some("attach") => {
            let id = payload
                .get("Id")
                .and_then(|v| v.as_str())
                .ok_or("attach: missing Id")?;
            start_attach(state, session, id, ws_tx).await
        }
        Some("pty_input") => {
            let input = payload
                .get("input")
                .and_then(|v| v.as_str())
                .ok_or("pty_input: missing input")?;
            let guard = session.lock().await;
            if let Some(ref tx) = guard.as_ref().and_then(|s| s.stdin_tx.as_ref()) {
                let mut w = tx.lock().await;
                use tokio::io::AsyncWriteExt;
                w.as_mut().write_all(input.as_bytes()).await.map_err(|e| e.to_string())?;
                w.as_mut().flush().await.map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        Some("pty_resize") => {
            let size = payload.get("size").ok_or("pty_resize: missing size")?;
            let rows = size.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
            let cols = size.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;
            let guard = session.lock().await;
            if let Some(ref s) = *guard {
                if let Some(ref exec_id) = s.exec_id {
                    let opts = ResizeExecOptions { height: rows, width: cols };
                    state
                        .docker
                        .resize_exec(exec_id, opts)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// 查詢容器是否為 running 狀態（inspect 的 state.status == RUNNING）。
fn container_running<'a>(
    docker: &'a bollard::Docker,
    id: &'a str,
) -> impl std::future::Future<Output = bool> + 'a {
    async move {
        let inspect = match docker.inspect_container(id, None).await {
            Ok(i) => i,
            Err(_) => return false,
        };
        inspect
            .state
            .as_ref()
            .and_then(|s| s.status.as_ref())
            .map(|st| matches!(st, ContainerStateStatusEnum::RUNNING))
            .unwrap_or(false)
    }
}

async fn start_shell(
    state: &AppState,
    session: &Arc<Mutex<Option<Session>>>,
    container_id: &str,
    ws_tx: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
) -> Result<(), String> {
    if !container_running(&state.docker, container_id).await {
        return Err("container not running".into());
    }
    let pid_path = format!("/tmp/_process_{}.pid", uuid::Uuid::new_v4());
    let create_opts = CreateExecOptions::<String> {
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        tty: Some(true),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("echo $$ > {}; exec /bin/bash", pid_path),
        ]),
        ..Default::default()
    };
    let create_res = state
        .docker
        .create_exec(container_id, create_opts)
        .await
        .map_err(|e| e.to_string())?;
    let exec_id = create_res.id.clone();
    let start_opts = StartExecOptions {
        detach: false,
        ..Default::default()
    };
    let start_res = state
        .docker
        .start_exec(&exec_id, Some(start_opts))
        .await
        .map_err(|e| e.to_string())?;
    let (output, input) = match start_res {
        bollard::exec::StartExecResults::Attached { output, input } => (output, input),
        bollard::exec::StartExecResults::Detached => return Err("exec detached".into()),
    };
    let stdin_tx = Arc::new(Mutex::new(Pin::from(input)));
    *session.lock().await = Some(Session {
        container_id: container_id.to_string(),
        exec_id: Some(exec_id.clone()),
        pid_path: Some(pid_path.clone()),
        stdin_tx: Some(stdin_tx.clone()),
    });
    tokio::spawn(forward_docker_stream_to_ws(output, ws_tx.clone()));
    Ok(())
}

async fn start_attach(
    state: &AppState,
    session: &Arc<Mutex<Option<Session>>>,
    container_id: &str,
    ws_tx: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
) -> Result<(), String> {
    if !container_running(&state.docker, container_id).await {
        return Err("container not running".into());
    }
    let opts = AttachContainerOptionsBuilder::default()
        .stdin(true)
        .stdout(true)
        .stderr(true)
        .stream(true)
        .logs(true)
        .build();
    let attach_res = state
        .docker
        .attach_container(container_id, Some(opts))
        .await
        .map_err(|e| e.to_string())?;
    let (output, input) = (attach_res.output, attach_res.input);
    *session.lock().await = Some(Session {
        container_id: container_id.to_string(),
        exec_id: None,
        pid_path: None,
        stdin_tx: Some(Arc::new(Mutex::new(Pin::from(input)))),
    });
    tokio::spawn(forward_docker_stream_to_ws(output, ws_tx.clone()));
    Ok(())
}

/// Forward bollard LogOutput stream to WebSocket text frames (decode as UTF-8; fallback lossy).
/// 在背景執行，不阻塞 recv 迴圈。串流結束時（例如 shell exit / Ctrl+D）主動關閉 WebSocket，讓前端收到 onclose。
async fn forward_docker_stream_to_ws<S>(
    mut stream: S,
    ws_tx: Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
) where
    S: futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Unpin + Send,
{
    while let Some(item) = stream.next().await {
        match item {
            Ok(LogOutput::StdOut { message })
            | Ok(LogOutput::StdErr { message })
            | Ok(LogOutput::Console { message }) => {
                let text = String::from_utf8_lossy(&message).to_string();
                let mut guard = ws_tx.lock().await;
                if guard.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
            Ok(LogOutput::StdIn { .. }) => {}
            Err(_) => break,
        }
    }
    // 串流結束（shell exit / attach 斷開）時關閉 WebSocket，前端才能顯示 [Connection closed]
    let mut guard = ws_tx.lock().await;
    let _ = guard
        .send(Message::Close(Some(CloseFrame {
            code: axum::extract::ws::close_code::NORMAL,
            reason: std::borrow::Cow::Borrowed("session ended"),
        })))
        .await;
}

async fn cleanup_shell(docker: &bollard::Docker, session: &Session) {
    let pid_path = match &session.pid_path {
        Some(p) => p.clone(),
        _ => return,
    };
    let kill_cmd = format!(
        "kill $(( $(cat {} 2>/dev/null) )) 2>/dev/null; rm -f {}",
        pid_path, pid_path
    );
    let opts = CreateExecOptions::<String> {
        attach_stdin: Some(false),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        tty: Some(false),
        cmd: Some(vec!["sh".to_string(), "-c".to_string(), kill_cmd]),
        ..Default::default()
    };
    if let Ok(create_res) = docker.create_exec(&session.container_id, opts).await {
        let _ = docker
            .start_exec(&create_res.id, Some(StartExecOptions::default()))
            .await;
    }
}
