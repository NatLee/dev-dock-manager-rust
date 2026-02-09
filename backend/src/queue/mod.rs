//! 背景任務佇列（Redis LPUSH/BRPOP）與通知廣播。
//! API 將「建立容器」等操作寫入佇列，worker 取出後執行 bollard 並透過 broadcast 通知 WebSocket 客戶端。

mod jobs;

use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const QUEUE_KEY: &str = "dev_dock_manager:queue";
const NOTIFY_CHANNEL: &str = "dev_dock_manager:notifications";

/// 單一任務種類：建立映像容器、啟動/停止/刪除/重啟容器。
#[derive(Clone, Serialize, Deserialize)]
pub enum Job {
    RunImage {
        image_name: String,
        ssh_port: u16,
        name: String,
        user: String,
        password: String,
        vnc_password: String,
        root_password: String,
        privileged: bool,
        nvdocker: bool,
        docker_network: String,
    },
    StartContainer { id: String },
    StopContainer { id: String },
    RemoveContainer { id: String },
    RestartContainer { id: String },
}

#[derive(Serialize, Deserialize)]
pub struct EnqueuedJob {
    pub task_id: String,
    pub job: Job,
}

fn new_task_id() -> String {
    format!(
        "{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

/// 將「建立並執行映像容器」任務寫入 Redis 佇列；回傳 task_id 供前端輪詢/通知。
pub async fn enqueue_run_image(
    redis_url: &str,
    docker_network: &str,
    image_name: &str,
    ssh_port: u16,
    name: &str,
    user: &str,
    password: &str,
    vnc_password: &str,
    root_password: &str,
    privileged: bool,
    nvdocker: bool,
) -> Result<String, String> {
    let client = redis::Client::open(redis_url).map_err(|e| e.to_string())?;
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| e.to_string())?;
    let task_id = new_task_id();
    let job = Job::RunImage {
        image_name: image_name.to_string(),
        ssh_port,
        name: name.to_string(),
        user: user.to_string(),
        password: password.to_string(),
        vnc_password: vnc_password.to_string(),
        root_password: root_password.to_string(),
        privileged,
        nvdocker,
        docker_network: docker_network.to_string(),
    };
    let payload = serde_json::to_string(&EnqueuedJob {
        task_id: task_id.clone(),
        job,
    })
    .map_err(|e| e.to_string())?;
    conn.lpush::<_, _, ()>(QUEUE_KEY, payload.as_str())
        .await
        .map_err(|e| e.to_string())?;
    Ok(task_id)
}

/// 向 Redis NOTIFY_CHANNEL 發送 WAITING 通知，供訂閱的 WebSocket 客戶端顯示。
pub async fn send_waiting_notification(redis_url: &str, container_id: &str, cmd: &str) {
    let payload = serde_json::json!({
        "action": "WAITING",
        "details": format!("Waiting [{}] for the task to complete [{}]", &container_id[..container_id.len().min(8)], cmd),
        "data": { "container_id": container_id, "cmd": cmd }
    });
    let msg = serde_json::json!({ "message": payload });
    if let Ok(client) = redis::Client::open(redis_url) {
        if let Ok(mut conn) = client.get_multiplexed_async_connection().await {
            let _: Result<(), _> = conn.publish(NOTIFY_CHANNEL, msg.to_string()).await;
        }
    }
}

/// 將啟動/停止/刪除/重啟容器任務寫入佇列；回傳 task_id。
pub async fn enqueue_containers_control(
    redis_url: &str,
    cmd: &str,
    id: &str,
) -> Option<String> {
    let job = match cmd {
        "start" => Job::StartContainer { id: id.to_string() },
        "stop" => Job::StopContainer { id: id.to_string() },
        "remove" => Job::RemoveContainer { id: id.to_string() },
        "restart" => Job::RestartContainer { id: id.to_string() },
        _ => return None,
    };
    let client = redis::Client::open(redis_url).ok()?;
    let mut conn = client.get_multiplexed_async_connection().await.ok()?;
    let task_id = new_task_id();
    let payload = serde_json::to_string(&EnqueuedJob {
        task_id: task_id.clone(),
        job,
    })
    .ok()?;
    conn.lpush::<_, _, ()>(QUEUE_KEY, payload.as_str()).await.ok()?;
    Some(task_id)
}

/// 向 Redis 頻道發送一則通知（CREATED/STARTED/STOPPED 等），供 WebSocket 訂閱者使用。
pub async fn publish_notification(redis_url: &str, action: &str, details: &str) {
    let payload = serde_json::json!({ "action": action, "details": details });
    let msg = serde_json::json!({ "message": payload });
    if let Ok(client) = redis::Client::open(redis_url) {
        if let Ok(mut conn) = client.get_multiplexed_async_connection().await {
            let _: Result<(), _> = conn.publish(NOTIFY_CHANNEL, msg.to_string()).await;
        }
    }
}

pub use jobs::run_worker;