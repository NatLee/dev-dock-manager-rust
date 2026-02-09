//! REST API 路由彙總：auth（JWT）、containers、images、ports。
//! 僅 JWT 登入，無 Google 等第三方登入路由。

mod auth;
mod containers;
mod images;
mod ports;

use axum::Router;

use crate::AppState;

/// 合併所有 REST 子路由，掛在 /api 下。
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(auth::router())
        .merge(containers::router())
        .merge(images::router())
        .merge(ports::router())
}
