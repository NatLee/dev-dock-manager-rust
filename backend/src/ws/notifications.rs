//! 通知 WebSocket：訂閱 app 的 broadcast channel，將佇列任務結果（CREATED/STARTED 等）轉發給連線中的客戶端。
//! 客戶端連上後僅接收 server 推送，不需送訊息。

use axum::{
    extract::{
        ws::Message,
        State,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};

use crate::AppState;

/// 接受 WebSocket 升級，訂閱 notify_tx，將收到的字串以 Text 幀轉發給客戶端。
pub async fn handler(
    State(state): State<AppState>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    let mut recv = state.notify_tx.subscribe();
    upgrade.on_upgrade(move |socket| async move {
        let (mut ws_sender, mut ws_recv) = socket.split();
        let send_task = async move {
            while let Ok(msg) = recv.recv().await {
                if ws_sender
                    .send(Message::Text(msg))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        };
        let recv_task = async move {
            while let Some(_) = ws_recv.next().await {
                // ignore incoming; we only push from server
            }
        };
        tokio::select! {
            _ = send_task => {}
            _ = recv_task => {}
        }
    })
}
