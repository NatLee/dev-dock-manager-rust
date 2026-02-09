//! 透過 bollard 操作 Docker：列容器/映像、解析埠、NVIDIA 檢測、console 元資料。
//! 與 Django 的容器/映像/埠邏輯對齊；僅處理使用 gui-vnc 前綴的映像。

pub mod nvidia;
pub mod ports;

use bollard::query_parameters::{
    InspectContainerOptionsBuilder, ListContainersOptionsBuilder, ListImagesOptionsBuilder,
};
use bollard::Docker;
use std::collections::HashMap;

use ports::parse_ports_bollard;

/// 映像 tag 前綴，用於篩選 GUI 容器/映像（與 Django DOCKER_IMAGE_NAME 一致）。
pub const GUI_IMAGE_TAG_PREFIX: &str = "gui-vnc";

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
}

/// 列出使用 gui-vnc 前綴映像的容器；每筆會 inspect 以取得名稱、狀態、埠等。
pub async fn list_containers_gui_vnc(
    docker: &Docker,
) -> Result<Vec<ContainerInfo>, bollard::errors::Error> {
    let opts = ListContainersOptionsBuilder::default()
        .all(true)
        .build();
    let summaries = docker.list_containers(Some(opts)).await?;
    let mut out = Vec::new();
    for c in summaries {
        let id = c.id.as_deref().unwrap_or("");
        let image = c.image.as_deref().unwrap_or("");
        // Image in summary can be tag or id; we need to inspect to get tags and filter.
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
        if !image_tag.starts_with(GUI_IMAGE_TAG_PREFIX) {
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
    /// All tags containing the image name (e.g. gui-vnc:latest), same as Django.
    pub tags: Vec<String>,
}

/// List images whose tags contain `image_name` (e.g. gui-vnc), matching Django ImagesListView.
pub async fn list_images(
    docker: &Docker,
    image_name: &str,
) -> Result<Vec<ImageInfo>, bollard::errors::Error> {
    let opts = ListImagesOptionsBuilder::default()
        .all(true)
        .build();
    let images = docker.list_images(Some(opts)).await?;
    let out = images
        .into_iter()
        .filter_map(|img| {
            let tags: &[String] = &img.repo_tags;
            let matching: Vec<String> = tags
                .iter()
                .filter(|t: &&String| t.contains(image_name))
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
