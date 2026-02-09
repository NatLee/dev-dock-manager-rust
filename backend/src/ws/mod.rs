//! WebSocket 路由：/ws/console（終端機）、/ws/notifications（任務狀態通知）。
//! 與 Django 的 ConsoleConsumer、通知推送對齊。

mod console;
mod notifications;

use axum::Router;

use crate::AppState;

/// 掛載 /ws/console 與 /ws/notifications（含尾端斜線以配合前端）。
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ws/console", axum::routing::get(console::handler))
        .route("/ws/console/", axum::routing::get(console::handler))
        .route("/ws/notifications", axum::routing::get(notifications::handler))
        .route("/ws/notifications/", axum::routing::get(notifications::handler))
}
