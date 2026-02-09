//! 從環境變數讀取設定（綁定位址、DB、Redis、JWT、Docker 網路等）。
//! 未使用 Google 登入，無 client id 等欄位。

#[derive(Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub redis_url: String,
    pub jwt_secret: String,
    pub docker_network: String,
    /// Host used for port-in-use check (e.g. host.docker.internal when running in Docker).
    pub host_for_port_check: String,
    /// Image name prefix for GUI containers (e.g. gui-vnc), same as Django DOCKER_IMAGE_NAME.
    pub docker_image_name: String,
}

impl Config {
    /// 從環境變數建構；未設定時使用預設值（如 8000、sqlite、redis://127.0.0.1 等）。
    pub fn from_env() -> Self {
        Self {
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8000".into()),
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:db.sqlite3".into()),
            redis_url: std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
            jwt_secret: std::env::var("JWT_SECRET").unwrap_or_else(|_| "change-me-in-production".into()),
            docker_network: std::env::var("DOCKER_NETWORK").unwrap_or_else(|_| "d-gui-network".into()),
            host_for_port_check: std::env::var("HOST_FOR_PORT_CHECK")
                .unwrap_or_else(|_| "host.docker.internal".into()),
            docker_image_name: std::env::var("DOCKER_IMAGE_NAME").unwrap_or_else(|_| "gui-vnc".into()),
        }
    }
}
