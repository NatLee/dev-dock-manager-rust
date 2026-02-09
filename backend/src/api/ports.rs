//! 埠位與環境檢查 API：取得可用埠、檢查埠是否被佔用、NVIDIA Docker 是否可用、是否為 Linux。
//! 對應 Django 的 free_ports、port_check、nvdocker、linux 等。

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::docker;
use crate::docker::ports;
use crate::AppState;

#[derive(Serialize)]
pub struct FreePortsResponse {
    pub free_ports: Vec<u16>,
}

#[derive(Serialize)]
pub struct PortCheckResponse {
    pub port: u16,
    pub is_used: bool,
}

#[derive(Serialize)]
pub struct NvidiaDockerResponse {
    pub nvidia_docker_available: bool,
}

#[derive(Serialize)]
pub struct LinuxCheckResponse {
    pub is_linux: bool,
}

#[derive(Deserialize)]
pub struct FreePortsQuery {
    #[serde(default = "default_count")]
    count: Option<u32>,
}

fn default_count() -> Option<u32> {
    Some(30)
}

#[derive(Deserialize)]
pub struct PortCheckQuery {
    port: Option<u16>,
}

async fn free_ports(
    State(state): State<AppState>,
    Query(q): Query<FreePortsQuery>,
) -> Result<Json<FreePortsResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let count = q.count.unwrap_or(30);
    if count == 0 {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Count must be a positive integer" })),
        ));
    }
    let free_ports = docker::find_multiple_free_ports(
        &state.docker,
        &state.config.host_for_port_check,
        count,
    )
    .await
    .map_err(|e| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
    })?;
    Ok(Json(FreePortsResponse { free_ports }))
}

async fn check_port(
    State(state): State<AppState>,
    Query(q): Query<PortCheckQuery>,
) -> Result<Json<PortCheckResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let port = q.port.ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Port parameter is missing" })),
        )
    })?;
    let in_use_host = ports::check_port_in_use(&state.config.host_for_port_check, port);
    let in_use_container = docker::is_port_used_by_container(&state.docker, port)
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?;
    Ok(Json(PortCheckResponse {
        port,
        is_used: in_use_host || in_use_container,
    }))
}

async fn nvdocker_check(
    State(state): State<AppState>,
) -> Result<Json<NvidiaDockerResponse>, (axum::http::StatusCode, Json<NvidiaDockerResponse>)> {
    let available = docker::nvidia::can_use_nvidia_docker(&state.docker).await;
    if available {
        Ok(Json(NvidiaDockerResponse {
            nvidia_docker_available: true,
        }))
    } else {
        Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(NvidiaDockerResponse {
                nvidia_docker_available: false,
            }),
        ))
    }
}

async fn linux_check() -> Json<LinuxCheckResponse> {
    Json(LinuxCheckResponse {
        is_linux: docker::is_linux(),
    })
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ports", get(free_ports))
        .route("/ports/check", get(check_port))
        .route("/nvdocker/check", get(nvdocker_check))
        .route("/linux/check", get(linux_check))
}
