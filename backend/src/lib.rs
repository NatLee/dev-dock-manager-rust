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
use std::path::PathBuf;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use config::Config;

/// 從 DATABASE_URL 解析出 SQLite 檔案的絕對路徑（僅限 file-based，排除 :memory:）。
/// 用於以 filename() 明確指定路徑，避免 URL 被解析成相對路徑導致 code 14。
fn sqlite_absolute_path(database_url: &str) -> Option<PathBuf> {
    let path_str = match database_url.strip_prefix("sqlite:///") {
        Some(rest) if !rest.is_empty() && rest != ":memory:" => format!("/{}", rest),
        _ => match database_url.strip_prefix("sqlite://") {
            Some(rest) if !rest.is_empty() && rest != ":memory:" => rest.to_string(),
            _ => match database_url.strip_prefix("sqlite:") {
                Some(rest) if !rest.is_empty() && rest != ":memory:" => {
                    if rest.starts_with('/') {
                        rest.to_string()
                    } else {
                        format!("/{}", rest)
                    }
                }
                _ => return None,
            },
        },
    };
    let path = PathBuf::from(&path_str);
    if path.as_os_str().is_empty() {
        return None;
    }
    Some(path)
}

/// 若 DATABASE_URL 為 sqlite 檔案路徑，先建立其父目錄，避免 SQLite code 14 (unable to open database file)。
fn ensure_sqlite_db_dir(database_url: &str) {
    if let Some(path) = sqlite_absolute_path(database_url) {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
    }
}

/// 依 DATABASE_URL 建立 SQLite pool：若為檔案 URL 則用絕對路徑 + SqliteConnectOptions 連線，避免 code 14。
async fn create_sqlite_pool(database_url: &str) -> Result<sqlx::SqlitePool, sqlx::Error> {
    if let Some(absolute_path) = sqlite_absolute_path(database_url) {
        ensure_sqlite_db_dir(database_url);
        let opts = SqliteConnectOptions::new()
            .filename(absolute_path)
            .create_if_missing(true);
        SqlitePoolOptions::new().connect_with(opts).await
    } else {
        // :memory: 或非檔案型 URL：沿用原本 URL 連線
        ensure_sqlite_db_dir(database_url);
        SqlitePoolOptions::new().connect(database_url).await
    }
}

/// 全域應用狀態：設定、DB pool、Docker 客戶端、通知廣播 sender。
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: sqlx::SqlitePool,
    pub docker: bollard::Docker,
    /// Broadcasts notification messages to WebSocket clients.
    pub notify_tx: tokio::sync::broadcast::Sender<String>,
}

/// 從環境變數載入設定、初始化 DB/migrations、Docker、Redis worker，組裝路由並啟動 HTTP server。
pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env();
    let pool = create_sqlite_pool(&config.database_url).await?;
    let migrations: std::path::PathBuf =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    sqlx::migrate::Migrator::new(migrations)
        .await?
        .run(&pool)
        .await?;

    let docker = docker::connect().map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        Box::from(e.to_string())
    })?;
    let (notify_tx, _) = tokio::sync::broadcast::channel::<String>(64);
        let app_state = AppState {
            config: config.clone(),
            pool,
            docker,
            notify_tx: notify_tx.clone(),
        };
        let redis_url = config.redis_url.clone();
        let docker_network = config.docker_network.clone();
        tokio::spawn(async move {
            queue::run_worker(redis_url, docker_network, notify_tx).await;
        });
        let app = router()
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_credentials(false),
            )
            .with_state(app_state);

        let addr: SocketAddr = config
            .bind_addr
            .parse()
        .unwrap_or_else(|_| "0.0.0.0:8000".parse().unwrap());
    tracing::info!("Rust API listening on {}", addr);
    axum::serve(
        tokio::net::TcpListener::bind(addr).await?,
        app,
    )
    .await?;
    Ok(())
}

/// 建立使用者 CLI（對應 Django 的 createsuperuser / 建立帳號腳本）。
/// 從環境變數讀取 DATABASE_URL，執行 migrations 後將給定的 username/password 寫入 users 表。
pub async fn create_user_cli(
    username: String,
    password: String,
    email: Option<String>,
    is_staff: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    let pool = create_sqlite_pool(&config.database_url).await?;
    let migrations: std::path::PathBuf =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    sqlx::migrate::Migrator::new(migrations)
        .await?
        .run(&pool)
        .await?;
    let hash = db::user::hash_password(&password)
        .map_err(|e| format!("hash_password failed: {}", e))?;
    let email_ref = email.as_deref();
    let id = db::user::create_user(&pool, &username, &hash, email_ref, is_staff).await?;
    println!("User created: id={} username={}", id, username);
    Ok(())
}

/// 組裝所有路由：/health、/api/*、/dashboard/api/*（同一 REST API）、WebSocket（/ws/console、/ws/notifications）。
fn router() -> Router<AppState> {
    Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .nest("/api", api::router())
        .nest("/dashboard/api", api::router())
        .merge(ws::router())
}
