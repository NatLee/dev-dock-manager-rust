//! 透過 bollard 操作 Docker：列容器/映像、解析埠、NVIDIA 檢測、console 元資料。
//! 與 Django 的容器/映像/埠邏輯對齊；只處理用設定裡的映像名稱（DOCKER_IMAGE_NAME）建立的容器。

pub mod nvidia;
pub mod ports;

use bollard::query_parameters::{
    InspectContainerOptionsBuilder, ListContainersOptionsBuilder, ListImagesOptionsBuilder,
};
use bollard::Docker;
use std::collections::HashMap;

use ports::parse_ports_bollard;

/// dev-dock 映像裡 noVNC 和 sshd 聽的 port（映像的 NO_VNC_PORT、SSH_PORT）。
pub const GUI_NOVNC_PORT: u16 = 6901;
pub const GUI_SSH_PORT: u16 = 22;

/// 記錄容器主人（使用者 id）的 label；Django 版用同一個名稱。
pub const OWNER_LABEL: &str = "dev-dock.owner";

/// 映像參照去掉 tag 或 digest 後的名稱：`gui-vnc:latest` → `gui-vnc`，
/// `registry:5000/team/gui-vnc:1` → `registry:5000/team/gui-vnc`。
pub fn image_repository(reference: &str) -> &str {
    let reference = reference.split('@').next().unwrap_or(reference);
    let last_segment_start = reference.rfind('/').map_or(0, |i| i + 1);
    match reference[last_segment_start..].rfind(':') {
        Some(i) => &reference[..last_segment_start + i],
        None => reference,
    }
}

/// 這個映像參照是不是 GUI 映像：名稱（不含 tag）完全等於設定的映像名稱。
pub fn is_gui_image(reference: &str, image_name: &str) -> bool {
    image_repository(reference) == image_name
}

/// 從容器 label 讀出主人的使用者 id；沒有或格式不對就是沒有主人。
pub fn owner_from_labels(labels: Option<&HashMap<String, String>>) -> Option<i64> {
    labels?.get(OWNER_LABEL)?.parse().ok()
}

/// 管理介面建立的容器要帶的 label：主人，以及讓 Traefik 把 /novnc/<name>/ 轉到它的 noVNC。
pub fn gui_container_labels(name: &str, docker_network: &str, novnc_port: u16, owner_id: i64) -> HashMap<String, String> {
    let router = format!("d-gui-{name}");
    HashMap::from([
        (OWNER_LABEL.to_string(), owner_id.to_string()),
        ("traefik.enable".to_string(), "true".to_string()),
        (format!("traefik.http.routers.{router}.rule"), format!("PathPrefix(`/novnc/{name}/`)")),
        (format!("traefik.http.services.{router}.loadbalancer.server.port"), novnc_port.to_string()),
        (format!("traefik.http.middlewares.{router}-strip-prefix.stripprefix.prefixes"), format!("/novnc/{name}/")),
        (format!("traefik.http.routers.{router}.middlewares"), format!("{router}-strip-prefix")),
        ("traefik.docker.network".to_string(), docker_network.to_string()),
    ])
}

/// 一個 GUI 容器的身分：id、名稱、主人、是否在跑。
#[derive(Debug, Clone)]
pub struct GuiContainer {
    pub id: String,
    pub name: String,
    pub owner_id: Option<i64>,
    pub running: bool,
}

/// 依 id 或名稱找 GUI 容器；不存在或不是用 GUI 映像建立的都回傳 None。
/// 所有「對某個容器做事」的 API 都先經過這裡，再用 User::can_access 判斷能不能碰。
pub async fn find_gui_container(
    docker: &Docker,
    id: &str,
    image_name: &str,
) -> Result<Option<GuiContainer>, bollard::errors::Error> {
    let inspect = match docker.inspect_container(id, None).await {
        Ok(i) => i,
        Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    let config = inspect.config.as_ref();
    if !config.and_then(|c| c.image.as_deref()).is_some_and(|img| is_gui_image(img, image_name)) {
        return Ok(None);
    }
    Ok(Some(GuiContainer {
        id: inspect.id.clone().unwrap_or_else(|| id.to_string()),
        name: inspect.name.as_deref().unwrap_or(id).trim_start_matches('/').to_string(),
        owner_id: owner_from_labels(config.and_then(|c| c.labels.as_ref())),
        running: inspect.state.as_ref().and_then(|s| s.running).unwrap_or(false),
    }))
}

/// 建立 Docker 連線（依環境 DOCKER_HOST / 本機預設）。
pub fn connect() -> Result<Docker, bollard::errors::Error> {
    Docker::connect_with_local_defaults()
}

/// 單一容器摘要，對應 Django API 回傳格式。
#[derive(serde::Serialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub command: Option<Vec<String>>,
    pub short_id: String,
    pub image_tag: String,
    pub ports: HashMap<String, String>,
    pub privileged: bool,
    pub nvdocker: bool,
    pub size_raw: i64,
    pub size_fs: i64,
    /// 主人的使用者 id（舊的、沒有 label 的容器是 None）
    pub owner_id: Option<i64>,
    /// 主人的帳號名稱，由 API 填入（只有管理員看得到別人的容器，所以只對管理員有用）
    pub owner: Option<String>,
}

/// 容器列表裡的一筆可能是桌面嗎（還沒 inspect 之前的便宜檢查）。
///
/// Docker 列出的是建立容器時用的映像名稱；那個 tag 之後被移到新的映像時，改列映像 ID。
/// 後者只有 inspect 看得出來（Config.Image 保留原本的名稱），所以先留著。
pub fn may_be_gui_container(listed_image: &str, image_id: Option<&str>, image_name: &str) -> bool {
    is_gui_image(listed_image, image_name) || image_id == Some(listed_image)
}

/// 列出用 GUI 映像建立、而且 `visible(主人的使用者 id)` 為真的容器。
///
/// 一次列出主機上所有容器，只 inspect（含大小）可能是桌面、這個人又看得到的那幾個。以前每個容器都
/// inspect 加計算大小，主機上無關的容器一多，列表就要一秒以上。
pub async fn list_gui_containers(
    docker: &Docker,
    image_name: &str,
    visible: impl Fn(Option<i64>) -> bool,
) -> Result<Vec<ContainerInfo>, bollard::errors::Error> {
    let opts = ListContainersOptionsBuilder::default()
        .all(true)
        .build();
    let summaries = docker.list_containers(Some(opts)).await?;
    let mut out = Vec::new();
    for c in summaries {
        let id = c.id.as_deref().unwrap_or("");
        let image = c.image.as_deref().unwrap_or("");
        if !may_be_gui_container(image, c.image_id.as_deref(), image_name)
            || !visible(owner_from_labels(c.labels.as_ref()))
        {
            continue;
        }
        // size=true is required for SizeRw and SizeRootFs to be returned
        let inspect_opts = InspectContainerOptionsBuilder::default()
            .size(true)
            .build();
        let inspect = match docker.inspect_container(id, Some(inspect_opts)).await {
            Ok(i) => i,
            Err(_) => continue,
        };
        let image_tag = inspect
            .config
            .as_ref()
            .and_then(|cfg| cfg.image.as_deref())
            .unwrap_or(image);
        if !is_gui_image(image_tag, image_name) {
            continue;
        }
        let name = inspect
            .name
            .as_deref()
            .map(|n| n.trim_start_matches('/'))
            .unwrap_or("")
            .to_string();
        let state = inspect
            .state
            .as_ref()
            .and_then(|s| s.status.as_ref())
            .map(|st| format!("{:?}", st).to_lowercase())
            .unwrap_or_else(|| "unknown".to_string());
        let host_config = inspect.host_config.as_ref();
        let port_bindings = host_config
            .and_then(|h| h.port_bindings.as_ref())
            .map(parse_ports_bollard)
            .unwrap_or_default();
        let privileged = host_config
            .map(|h| h.privileged.unwrap_or(false))
            .unwrap_or(false);
        let nvdocker = host_config
            .and_then(|h| h.device_requests.as_ref())
            .map(|dr| {
                dr.iter()
                    .any(|r| r.driver.as_deref() == Some("nvidia"))
            })
            .unwrap_or(false);
        let short_id = id.chars().take(12).collect::<String>();
        out.push(ContainerInfo {
            id: id.to_string(),
            name,
            status: state,
            command: inspect.config.as_ref().and_then(|c| c.cmd.clone()),
            short_id,
            image_tag: image_tag.to_string(),
            ports: port_bindings,
            privileged,
            nvdocker,
            size_raw: inspect.size_rw.unwrap_or(0),
            size_fs: inspect.size_root_fs.unwrap_or(0),
            owner_id: owner_from_labels(inspect.config.as_ref().and_then(|c| c.labels.as_ref())),
            owner: None,
        });
    }
    Ok(out)
}

/// Image list item matching Django API response shape.
#[derive(serde::Serialize)]
pub struct ImageInfo {
    pub id: String,
    pub size: f64,
    pub short_id: String,
    pub name: Option<String>,
    /// All tags of this image that are named after the GUI image (e.g. gui-vnc:latest).
    pub tags: Vec<String>,
}

/// List images that have at least one tag named `image_name` (e.g. gui-vnc:latest).
pub async fn list_images(
    docker: &Docker,
    image_name: &str,
) -> Result<Vec<ImageInfo>, bollard::errors::Error> {
    // Docker filters by repository name (any tag), so only the GUI image comes back.
    let opts = ListImagesOptionsBuilder::default()
        .filters(&HashMap::from([("reference", vec![image_name])]))
        .build();
    let images = docker.list_images(Some(opts)).await?;
    let out = images
        .into_iter()
        .filter_map(|img| {
            let tags: &[String] = &img.repo_tags;
            let matching: Vec<String> = tags
                .iter()
                .filter(|t: &&String| is_gui_image(t, image_name))
                .cloned()
                .collect();
            if matching.is_empty() {
                return None;
            }
            let id_str: &str = img.id.as_str();
            let size_mb = (img.size as f64) / 1_048_576.0;
            let short_id = id_str.chars().skip(7).take(12).collect::<String>();
            let name = matching.first().cloned();
            Some(ImageInfo {
                id: id_str.chars().skip(7).collect::<String>(),
                size: (size_mb * 100.0).round() / 100.0,
                short_id,
                name,
                tags: matching,
            })
        })
        .collect();
    Ok(out)
}

/// Get Docker system info (for /api/images "info" field).
pub async fn system_info(
    docker: &Docker,
) -> Result<serde_json::Value, String> {
    let info = docker.info().await.map_err(|e| e.to_string())?;
    serde_json::to_value(info).map_err(|e| e.to_string())
}

/// Check if any container is using the given host port.
pub async fn is_port_used_by_container(
    docker: &Docker,
    port: u16,
) -> Result<bool, bollard::errors::Error> {
    let opts = ListContainersOptionsBuilder::default()
        .all(true)
        .build();
    let summaries = docker.list_containers(Some(opts)).await?;
    let port_str = port.to_string();
    for c in summaries {
        let id = match &c.id { Some(x) => x.as_str(), None => continue };
        let inspect = match docker.inspect_container(id, None).await {
            Ok(i) => i,
            Err(_) => continue,
        };
        let bindings = inspect
            .host_config
            .as_ref()
            .and_then(|h| h.port_bindings.as_ref());
        if let Some(pb) = bindings {
            let parsed = parse_ports_bollard(pb);
            if parsed.values().any(|v| v == &port_str) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Find up to `count` free ports (not in use on host and not used by any container).
pub async fn find_multiple_free_ports(
    docker: &Docker,
    host: &str,
    count: u32,
) -> Result<Vec<u16>, String> {
    let mut out = Vec::with_capacity(count as usize);
    let mut port: u16 = 1024;
    while (out.len() as u32) < count && port < 65535 {
        if !ports::check_port_in_use(host, port)
            && !is_port_used_by_container(docker, port)
                .await
                .map_err(|e| e.to_string())?
        {
            out.push(port);
        }
        port += 1;
    }
    if (out.len() as u32) < count {
        return Err("Not enough free ports available.".to_string());
    }
    Ok(out)
}

/// Is the host OS Linux?
pub fn is_linux() -> bool {
    cfg!(target_os = "linux")
}

/// Console meta for GET /api/console/:action/:id.
#[derive(serde::Serialize)]
pub struct ConsoleMeta {
    pub id: String,
    pub container_name: String,
    pub image: String,
    pub short_id: String,
    pub command: Option<String>,
    pub action: String,
}

pub async fn get_console_meta(
    docker: &Docker,
    id: &str,
    action: &str,
) -> Result<ConsoleMeta, bollard::errors::Error> {
    let inspect = docker.inspect_container(id, None).await?;
    let name = inspect
        .name
        .as_deref()
        .map(|n| n.trim_start_matches('/').to_string())
        .unwrap_or_else(|| id.to_string());
    let image = inspect
        .config
        .as_ref()
        .and_then(|c| c.image.as_deref())
        .unwrap_or("")
        .to_string();
    let short_id = inspect
        .id
        .as_deref()
        .map(|s| s.chars().take(12).collect::<String>())
        .unwrap_or_else(|| id.chars().take(12).collect());
    let command = inspect
        .config
        .as_ref()
        .and_then(|c| c.cmd.as_ref())
        .map(|cmd| cmd.join(" "));
    Ok(ConsoleMeta {
        id: id.to_string(),
        container_name: name,
        image,
        short_id,
        command,
        action: action.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_repository_drops_tag_and_digest_only() {
        assert_eq!(image_repository("gui-vnc"), "gui-vnc");
        assert_eq!(image_repository("gui-vnc:latest"), "gui-vnc");
        assert_eq!(image_repository("gui-vnc@sha256:abc"), "gui-vnc");
        assert_eq!(image_repository("natlee/gui-vnc:2.1.0"), "natlee/gui-vnc");
        assert_eq!(image_repository("registry:5000/team/gui-vnc:1"), "registry:5000/team/gui-vnc");
    }

    #[test]
    fn gui_image_needs_the_exact_name() {
        assert!(is_gui_image("gui-vnc:latest", "gui-vnc"));
        assert!(!is_gui_image("dev-dock-gui-vnc:e2e", "gui-vnc"));
        assert!(!is_gui_image("gui-vnc-old:1", "gui-vnc"));
        assert!(!is_gui_image("natlee/gui-vnc:latest", "gui-vnc"));
    }

    #[test]
    fn labels_carry_the_owner_and_the_novnc_route() {
        let labels = gui_container_labels("alice", "d-gui-network", GUI_NOVNC_PORT, 7);
        assert_eq!(owner_from_labels(Some(&labels)), Some(7));
        assert_eq!(labels["traefik.http.routers.d-gui-alice.rule"], "PathPrefix(`/novnc/alice/`)");
        assert_eq!(labels["traefik.http.services.d-gui-alice.loadbalancer.server.port"], "6901");
        assert_eq!(labels["traefik.docker.network"], "d-gui-network");
    }

    #[test]
    fn only_possible_desktops_get_inspected() {
        let id = "sha256:".to_string() + &"f".repeat(64);
        assert!(may_be_gui_container("gui-vnc", Some(&id), "gui-vnc"));
        assert!(may_be_gui_container("gui-vnc:2.0.1", Some(&id), "gui-vnc"));
        // the tag moved to a newer image: Docker lists the ID, the inspect decides
        assert!(may_be_gui_container(&id, Some(&id), "gui-vnc"));
        assert!(!may_be_gui_container("traefik:v3.7", Some(&id), "gui-vnc"));
        assert!(!may_be_gui_container("natlee/gui-vnc:latest", Some(&id), "gui-vnc"));
    }

    #[test]
    fn missing_or_bad_owner_label_means_no_owner() {
        assert_eq!(owner_from_labels(None), None);
        let labels = HashMap::from([(OWNER_LABEL.to_string(), "not-a-number".to_string())]);
        assert_eq!(owner_from_labels(Some(&labels)), None);
    }
}
