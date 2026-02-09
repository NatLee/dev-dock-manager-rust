//! 容器 REST API：列出 GUI 容器、建立（丟進佇列）、啟動/停止/刪除/重啟、取得 console 元資料。
//! 與 Django xterm views 對齊（list、run、control、console meta）。

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth_extractor::AuthUser;
use crate::docker;
use crate::docker::ports;
use crate::AppState;

#[derive(Serialize)]
pub struct ContainersResponse {
    pub containers: Vec<docker::ContainerInfo>,
}

async fn list_containers(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<ContainersResponse>, (axum::http::StatusCode, String)> {
    let containers = docker::list_containers_gui_vnc(&state.docker)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(ContainersResponse { containers }))
}

async fn console_meta(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((action, id)): Path<(String, String)>,
) -> Result<Json<docker::ConsoleMeta>, (axum::http::StatusCode, String)> {
    let _ = auth;
    let meta = docker::get_console_meta(&state.docker, &id, &action)
        .await
        .map_err(|e| (axum::http::StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(meta))
}

#[derive(Deserialize)]
pub struct RunContainerBody {
    pub container_name: String,
    pub ssh: String,
    pub user: String,
    pub password: String,
    pub vnc_password: String,
    pub root_password: String,
    #[serde(default)]
    pub privileged: bool,
    #[serde(default)]
    pub nvdocker: bool,
}

#[derive(Serialize)]
pub struct RunContainerResponse {
    pub container_name: String,
    pub task_id: String,
}

async fn run_container(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<RunContainerBody>,
) -> Result<Json<RunContainerResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let _ = auth;
    let name = body.container_name.replace('/', "-");
    if name.len() < 2 {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Container name must be at least 2 characters long" })),
        ));
    }
    if !name.chars().next().map_or(false, |c| c.is_ascii_alphabetic()) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Container name must start with a letter [a-zA-Z]" })),
        ));
    }
    let ssh_port: u16 = body.ssh.parse().map_err(|_| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Non-integer value provided" })),
        )
    })?;
    if docker::is_port_used_by_container(&state.docker, ssh_port)
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?
    {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Port [{}] is already in use by container", ssh_port) })),
        ));
    }
    if ports::check_port_in_use(&state.config.host_for_port_check, ssh_port) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Port [{}] is already in use by other services", ssh_port) })),
        ));
    }
    let task_id = crate::queue::enqueue_run_image(
        &state.config.redis_url,
        &state.config.docker_network,
        docker::GUI_IMAGE_TAG_PREFIX,
        ssh_port,
        &name,
        &body.user,
        &body.password,
        &body.vnc_password,
        &body.root_password,
        body.privileged,
        body.nvdocker,
    )
    .await
    .map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(RunContainerResponse {
        container_name: name,
        task_id,
    }))
}

#[derive(Deserialize)]
pub struct ContainersControlBody {
    pub cmd: String,
    pub id: String,
}

#[derive(Serialize)]
pub struct ContainersControlResponse {
    pub task_id: Option<String>,
}

async fn containers_control(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<ContainersControlBody>,
) -> Result<Json<ContainersControlResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let _ = auth;
    let valid = ["start", "stop", "restart", "remove"];
    if !valid.contains(&body.cmd.as_str()) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid cmd" })),
        ));
    }
    let task_id = if ["start", "restart", "stop", "remove"].contains(&body.cmd.as_str()) {
        let waiting_msg = serde_json::json!({
            "message": {
                "action": "WAITING",
                "details": format!("Waiting [{}] for the task to complete [{}]", &body.id[..body.id.len().min(8)], body.cmd),
                "data": { "container_id": &body.id, "cmd": &body.cmd }
            }
        });
        let _ = state.notify_tx.send(waiting_msg.to_string());
        crate::queue::enqueue_containers_control(&state.config.redis_url, &body.cmd, &body.id)
            .await
    } else {
        None
    };
    Ok(Json(ContainersControlResponse { task_id }))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/containers", get(list_containers))
        .route("/console/:action/:id", get(console_meta))
        .route("/container/new", post(run_container))
        .route("/containers/control", post(containers_control))
}
