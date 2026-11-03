//! 容器 REST API：列出 GUI 容器、建立（丟進佇列）、啟動/停止/刪除/重啟、取得 console 元資料。
//! 與 Django xterm views 對齊（list、run、control、console meta）。
//!
//! 權限：一般使用者只看得到、只能操作自己建立的容器（owner label）；管理員全部都可以。
//! 不能碰的容器一律回 404，不透露它存在。只有管理員能建立 privileged 容器。

use std::collections::{HashMap, HashSet};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::auth_extractor::AuthUser;
use crate::db::User;
use crate::docker::{self, ports, GuiContainer};
use crate::queue::{self, Job, NewContainer, Notification};
use crate::AppState;
use super::{api_error, ApiError};

/// 找出使用者可以操作的 GUI 容器；找不到或沒權限都回 404。
pub async fn accessible_container(state: &AppState, user: &User, id: &str) -> Result<GuiContainer, ApiError> {
    docker::find_gui_container(&state.docker, id, &state.config.docker_image_name)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .filter(|c| user.can_access(c.owner_id))
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Container not found"))
}

#[derive(Serialize)]
pub struct ContainersResponse {
    pub containers: Vec<docker::ContainerInfo>,
}

async fn list_containers(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
) -> Result<Json<ContainersResponse>, ApiError> {
    let mut containers: Vec<_> = docker::list_gui_containers(&state.docker, &state.config.docker_image_name)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .into_iter()
        .filter(|c| user.can_access(c.owner_id))
        .collect();
    // 填上主人的帳號名稱（管理員才會看到別人的容器）
    let owner_ids: HashSet<i64> = containers.iter().filter_map(|c| c.owner_id).collect();
    let mut names = HashMap::new();
    for id in owner_ids {
        if let Ok(Some(owner)) = crate::db::get_by_id(&state.pool, id).await {
            names.insert(id, owner.username);
        }
    }
    for c in &mut containers {
        c.owner = c.owner_id.and_then(|id| names.get(&id).cloned());
    }
    Ok(Json(ContainersResponse { containers }))
}

async fn console_meta(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Path((action, id)): Path<(String, String)>,
) -> Result<Json<docker::ConsoleMeta>, ApiError> {
    let container = accessible_container(&state, &user, &id).await?;
    let meta = docker::get_console_meta(&state.docker, &container.id, &action)
        .await
        .map_err(|e| api_error(StatusCode::NOT_FOUND, e))?;
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
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<RunContainerBody>,
) -> Result<Json<RunContainerResponse>, ApiError> {
    if body.privileged && !user.is_staff {
        return Err(api_error(StatusCode::FORBIDDEN, "Only admins can create privileged containers"));
    }
    let name = body.container_name.replace('/', "-");
    if name.len() < 2 {
        return Err(api_error(StatusCode::BAD_REQUEST, "Container name must be at least 2 characters long"));
    }
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return Err(api_error(StatusCode::BAD_REQUEST, "Container name must start with a letter [a-zA-Z]"));
    }
    let ssh_port: u16 = body
        .ssh
        .parse()
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Non-integer value provided"))?;
    if docker::is_port_used_by_container(&state.docker, ssh_port)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("Port [{}] is already in use by container", ssh_port),
        ));
    }
    if ports::check_port_in_use(&state.config.host_for_port_check, ssh_port) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("Port [{}] is already in use by other services", ssh_port),
        ));
    }
    let spec = NewContainer {
        image_name: state.config.docker_image_name.clone(),
        name: name.clone(),
        ssh_port,
        user: body.user,
        password: body.password,
        vnc_password: body.vnc_password,
        root_password: body.root_password,
        privileged: body.privileged,
        nvdocker: body.nvdocker,
        docker_network: state.config.docker_network.clone(),
        owner_id: user.id,
    };
    let task_id = queue::enqueue(&state.config.redis_url, Some(user.id), Job::RunImage(spec))
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
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
    pub task_id: String,
}

async fn containers_control(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Json(body): Json<ContainersControlBody>,
) -> Result<Json<ContainersControlResponse>, ApiError> {
    let container = accessible_container(&state, &user, &body.id).await?;
    let job = Job::control(&body.cmd, &container.id).ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "invalid cmd"))?;
    let short_id: String = body.id.chars().take(8).collect();
    let _ = state.notify_tx.send(Notification::new(
        container.owner_id,
        "WAITING",
        &format!("Waiting [{}] for the task to complete [{}]", short_id, body.cmd),
        Some(serde_json::json!({ "container_id": body.id, "cmd": body.cmd })),
    ));
    let task_id = queue::enqueue(&state.config.redis_url, container.owner_id, job)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(ContainersControlResponse { task_id }))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/containers", get(list_containers))
        .route("/console/:action/:id", get(console_meta))
        .route("/container/new", post(run_container))
        .route("/containers/control", post(containers_control))
}
