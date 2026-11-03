//! REST API 路由彙總：auth（JWT）、users（帳號管理）、containers、images、ports。
//! 僅 JWT 登入，無 Google 等第三方登入路由。

mod auth;
pub mod containers;
mod images;
mod ports;
mod users;

use axum::{http::StatusCode, Json, Router};

use crate::AppState;

/// 合併所有 REST 子路由，掛在 /api 下。
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(auth::router())
        .merge(containers::router())
        .merge(images::router())
        .merge(ports::router())
        .merge(users::router())
}

/// Error answer of the JSON API: an HTTP status and `{"error": "..."}` (what the dashboard shows).
pub(crate) type ApiError = (StatusCode, Json<serde_json::Value>);

pub(crate) fn api_error(status: StatusCode, message: impl ToString) -> ApiError {
    (status, Json(serde_json::json!({ "error": message.to_string() })))
}
