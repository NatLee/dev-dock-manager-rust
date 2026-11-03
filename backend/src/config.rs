//! 從環境變數讀取設定（綁定位址、DB、Redis、JWT、Docker 網路等）。
//! 未使用 Google 登入，無 client id 等欄位。

#[derive(Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub redis_url: String,
    /// 簽 JWT 用的密鑰。沒有預設值：用公開的預設值簽，等於誰都能自己做 token。
    pub jwt_secret: String,
    pub docker_network: String,
    /// Host used for port-in-use check (e.g. host.docker.internal when running in Docker).
    pub host_for_port_check: String,
    /// GUI 容器用的映像名稱（例如 gui-vnc）：建立容器、列出容器和映像都用這個名稱。
    pub docker_image_name: String,
    /// GPU 檢查執行 nvidia-smi 用的映像（NVIDIA_CHECK_IMAGE，和 Django 版同名）。
    pub nvidia_check_image: String,
}

/// SQLite 的位置；伺服器和命令列的 create-user 共用。
pub fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:db.sqlite3".into())
}

impl Config {
    /// 從環境變數建構；未設定時使用預設值（如 8000、sqlite、redis://127.0.0.1 等）。
    /// `JWT_SECRET` 一定要設，沒設就回傳錯誤，讓程式在啟動時就清楚地停下來。
    pub fn from_env() -> Result<Self, String> {
        let var = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
        let jwt_secret = std::env::var("JWT_SECRET")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .ok_or("JWT_SECRET is not set. Put a long random string in it (see .env.example).")?;
        Ok(Self {
            bind_addr: var("BIND_ADDR", "0.0.0.0:8000"),
            database_url: database_url(),
            redis_url: var("REDIS_URL", "redis://127.0.0.1:6379"),
            jwt_secret,
            docker_network: var("DOCKER_NETWORK", "d-gui-network"),
            host_for_port_check: var("HOST_FOR_PORT_CHECK", "host.docker.internal"),
            docker_image_name: var("DOCKER_IMAGE_NAME", "gui-vnc"),
            nvidia_check_image: var("NVIDIA_CHECK_IMAGE", "nvidia/cuda:11.0.3-base-ubuntu20.04"),
        })
    }
}
