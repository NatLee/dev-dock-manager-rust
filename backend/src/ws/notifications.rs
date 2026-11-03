//! 通知 WebSocket：把佇列任務的結果（CREATED/STARTED 等）轉給儀表板。
//!
//! 連上後第一則訊息要帶 access token（`{"token": "..."}`，和 console 一樣，不放進 URL）。
//! 驗證通過後只轉送使用者看得到的容器的通知：自己的，管理員則是全部。

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use crate::db::User;
use crate::AppState;

pub async fn handler(
    State(state): State<AppState>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| handle_socket(socket, state))
}

/// 等第一則訊息裡的 token，換成使用者；失敗就回 None。
async fn authenticate_first_message(socket: &mut WebSocket, state: &AppState) -> Option<User> {
    let Some(Ok(Message::Text(text))) = socket.next().await else {
        return None;
    };
    let token = serde_json::from_str::<serde_json::Value>(&text).ok()?.get("token")?.as_str()?.to_string();
    crate::auth_extractor::authenticate(state, &token).await.ok()
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    // 先訂閱再驗證，驗證期間發生的通知也不會漏掉。
    let mut recv = state.notify_tx.subscribe();
    let Some(user) = authenticate_first_message(&mut socket, &state).await else {
        let _ = socket.send(Message::Close(super::close_unauthorized())).await;
        return;
    };
    let (mut ws_sender, mut ws_recv) = socket.split();
    let send_task = async move {
        loop {
            let note = match recv.recv().await {
                Ok(note) => note,
                // 太慢跟不上時只會漏掉舊的通知，連線繼續用
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            if !user.can_access(note.owner_id) {
                continue;
            }
            if ws_sender.send(Message::Text(note.payload)).await.is_err() {
                break;
            }
        }
    };
    let recv_task = async move {
        while ws_recv.next().await.is_some() {
            // ignore incoming; we only push from server
        }
    };
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
}
