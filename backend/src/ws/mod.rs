//! WebSocket 路由：/ws/console（終端機）、/ws/notifications（任務狀態通知）。
//! 兩者都在第一則訊息驗證 access token。
//! 與 Django 的 ConsoleConsumer、通知推送對齊。

mod console;
mod notifications;

use axum::extract::ws::{close_code, CloseFrame};
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

/// 驗證失敗時關閉 WebSocket 用的 frame（console 和 notifications 共用）。
pub(crate) fn close_unauthorized() -> Option<CloseFrame<'static>> {
    Some(CloseFrame {
        code: close_code::POLICY,
        reason: std::borrow::Cow::Borrowed("Unauthorized"),
    })
}
