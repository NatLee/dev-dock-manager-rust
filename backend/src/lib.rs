//! 後端程式庫：路由、設定與應用啟動。
//!
//! 職責：從環境載入設定、建立 DB 連線與 migrations、Docker 連線、Redis 佇列 worker、
//! 廣播 channel，組裝 Axum router（/health、/api/*、WebSocket）並啟動 HTTP server。
//! 本專案不使用 Google 登入，僅 JWT。

pub mod api;
pub mod auth_extractor;
pub mod config;
pub mod docker;
pub mod jwt;
pub mod queue;
pub mod ws;

pub mod db;

use axum::Router;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::net::SocketAddr;
use std::str::FromStr;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use config::Config;
use queue::Notification;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// 一次最多暫存幾則還沒送出的通知；跟不上的 WebSocket 只會漏掉舊的（見 ws::notifications）。
const NOTIFICATION_BUFFER: usize = 64;

/// 依 DATABASE_URL 連上 SQLite（檔案不存在就建立，含上層資料夾），並套用 migrations。
/// migrations 在編譯時就包進執行檔，執行時不需要原始碼目錄。
pub async fn connect_db(database_url: &str) -> Result<sqlx::SqlitePool, BoxError> {
    let options = SqliteConnectOptions::from_str(database_url)?.create_if_missing(true);
    let file = options.get_filename();
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        if file.to_str() != Some(":memory:") {
            std::fs::create_dir_all(dir)?;
        }
    }
    let pool = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// 全域應用狀態：設定、DB pool、Docker 客戶端、通知廣播 sender。
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: sqlx::SqlitePool,
    pub docker: bollard::Docker,
    /// Broadcasts notifications (with the container owner) to WebSocket clients.
    pub notify_tx: tokio::sync::broadcast::Sender<Notification>,
}

impl AppState {
    pub fn new(config: Config, pool: sqlx::SqlitePool, docker: bollard::Docker) -> Self {
        let (notify_tx, _) = tokio::sync::broadcast::channel(NOTIFICATION_BUFFER);
        AppState { config, pool, docker, notify_tx }
    }
}

/// 組裝所有路由：/health、/api/*、/dashboard/api/*（同一 REST API）、WebSocket（/ws/console、/ws/notifications）。
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .nest("/api", api::router())
        .nest("/dashboard/api", api::router())
        .merge(ws::router())
        .layer(CorsLayer::new().allow_origin(Any).allow_credentials(false))
        .with_state(state)
}

/// 從環境變數載入設定、初始化 DB/migrations、Docker、Redis worker，組裝路由並啟動 HTTP server。
pub async fn run() -> Result<(), BoxError> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;
    let pool = connect_db(&config.database_url).await?;
    let docker = docker::connect()?;
    let state = AppState::new(config.clone(), pool, docker);

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(queue::run_worker(
        config.redis_url.clone(),
        state.notify_tx.clone(),
        shutdown_rx,
    ));

    let addr: SocketAddr = config
        .bind_addr
        .parse()
        .map_err(|e| format!("BIND_ADDR {:?} is not an address: {}", config.bind_addr, e))?;
    tracing::info!("Rust API listening on {}", addr);
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // HTTP 已停止；通知 worker 不再取新任務，並等它做完手上那一個（docker stop 預設只給 10 秒）。
    let _ = shutdown_tx.send(true);
    if tokio::time::timeout(WORKER_SHUTDOWN_GRACE, worker).await.is_err() {
        tracing::warn!(
            "Queue worker still busy after {:?}; exiting anyway",
            WORKER_SHUTDOWN_GRACE
        );
    }
    tracing::info!("Rust API stopped");
    Ok(())
}

/// 收到終止訊號後，最多等 worker 多久（要小於 docker stop 預設的 10 秒）。
const WORKER_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// 等待終止訊號：SIGINT（Ctrl+C）或 SIGTERM（`docker stop`、Kubernetes 停 Pod 時送的）。
///
/// 收到後 axum 會停止接受新連線，等進行中的請求處理完再讓 `run()` 返回。
/// 在容器裡後台是 PID 1，沒有自己處理 SIGTERM 的話訊號會被忽略，只能等 runtime 改送 SIGKILL。
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("Cannot listen for SIGINT: {}", e);
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(e) => {
                tracing::error!("Cannot listen for SIGTERM: {}", e);
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received SIGINT, shutting down"),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down"),
    }
}

/// 建立使用者 CLI（對應 Django 的 createsuperuser / 建立帳號腳本）。
/// 只需要 DATABASE_URL；執行 migrations 後將給定的 username/password 寫入 users 表。
/// 和網頁不同，這裡不檢查密碼長度，方便第一次建立管理員。
pub async fn create_user_cli(
    username: String,
    password: String,
    email: Option<String>,
    is_staff: bool,
) -> Result<(), BoxError> {
    dotenvy::dotenv().ok();
    let pool = connect_db(&config::database_url()).await?;
    let hash = db::user::hash_password(&password)
        .map_err(|e| format!("hash_password failed: {}", e))?;
    let id = db::user::create_user(&pool, &username, &hash, email.as_deref(), is_staff).await?;
    println!("User created: id={} username={}", id, username);
    Ok(())
}
