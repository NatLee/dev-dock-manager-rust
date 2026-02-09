//! 映像列表 API：依名稱前綴（如 gui-vnc）過濾並回傳映像清單與系統資訊。
//! 對應 Django 的映像列表。

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;

use crate::auth_extractor::AuthUser;
use crate::docker;
use crate::AppState;

#[derive(Serialize)]
pub struct ImagesResponse {
    pub images: Vec<docker::ImageInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<serde_json::Value>,
}

async fn list_images(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<ImagesResponse>, (axum::http::StatusCode, String)> {
    let images = docker::list_images(&state.docker, &state.config.docker_image_name)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let info = docker::system_info(&state.docker).await.ok();
    Ok(Json(ImagesResponse { images, info }))
}

/// GET /images：需 JWT，回傳符合前綴的映像與可選 system info。
pub fn router() -> Router<AppState> {
    Router::new().route("/images", get(list_images))
}
