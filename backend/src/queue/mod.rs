//! 背景任務佇列（Redis LPUSH/BRPOP）與通知。
//! API 將「建立容器」等操作寫入佇列，worker 取出後執行 bollard，並把結果當成通知廣播給 WebSocket；
//! 每則通知帶著容器主人，WebSocket 只轉給主人和管理員。

mod jobs;

use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const QUEUE_KEY: &str = "dev_dock_manager:queue";

/// 建立一個 GUI 容器需要的全部資料。
#[derive(Clone, Serialize, Deserialize)]
pub struct NewContainer {
    pub image_name: String,
    pub name: String,
    pub ssh_port: u16,
    pub user: String,
    pub password: String,
    pub vnc_password: String,
    pub root_password: String,
    pub privileged: bool,
    pub nvdocker: bool,
    pub docker_network: String,
    /// 建立者的使用者 id，寫進容器的 owner label。
    pub owner_id: i64,
}

/// 單一任務種類：建立容器、啟動/停止/刪除/重啟容器。
#[derive(Clone, Serialize, Deserialize)]
pub enum Job {
    RunImage(NewContainer),
    StartContainer { id: String },
    StopContainer { id: String },
    RemoveContainer { id: String },
    RestartContainer { id: String },
}

impl Job {
    /// 從儀表板的指令（start/stop/restart/remove）建立對應的任務。
    pub fn control(cmd: &str, id: &str) -> Option<Self> {
        let id = id.to_string();
        match cmd {
            "start" => Some(Job::StartContainer { id }),
            "stop" => Some(Job::StopContainer { id }),
            "remove" => Some(Job::RemoveContainer { id }),
            "restart" => Some(Job::RestartContainer { id }),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct EnqueuedJob {
    pub task_id: String,
    /// 任務所屬容器的主人，用來決定結果通知要送給誰。
    #[serde(default)]
    pub owner_id: Option<i64>,
    pub job: Job,
}

/// 一則給儀表板的通知：`{"message": {"action", "details", "data"?}}`，加上容器主人。
#[derive(Clone, Debug)]
pub struct Notification {
    pub owner_id: Option<i64>,
    pub payload: String,
}

impl Notification {
    pub fn new(owner_id: Option<i64>, action: &str, details: &str, data: Option<serde_json::Value>) -> Self {
        let mut message = serde_json::json!({ "action": action, "details": details });
        if let Some(data) = data {
            message["data"] = data;
        }
        Notification {
            owner_id,
            payload: serde_json::json!({ "message": message }).to_string(),
        }
    }
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

/// 把任務寫入 Redis 佇列；回傳 task_id 供前端對照。
pub async fn enqueue(redis_url: &str, owner_id: Option<i64>, job: Job) -> Result<String, String> {
    let client = redis::Client::open(redis_url).map_err(|e| e.to_string())?;
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| e.to_string())?;
    let task_id = new_task_id();
    let payload = serde_json::to_string(&EnqueuedJob {
        task_id: task_id.clone(),
        owner_id,
        job,
    })
    .map_err(|e| e.to_string())?;
    conn.lpush::<_, _, ()>(QUEUE_KEY, payload.as_str())
        .await
        .map_err(|e| e.to_string())?;
    Ok(task_id)
}

pub use jobs::run_worker;
