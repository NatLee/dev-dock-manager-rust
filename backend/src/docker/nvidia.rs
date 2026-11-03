//! 檢測本機是否可用 NVIDIA Docker（nvidia runtime）。
//! 用設定的映像（NVIDIA_CHECK_IMAGE）建立一個暫存容器執行 nvidia-smi 來判斷。

use bollard::models::ContainerCreateBody;
use bollard::query_parameters::CreateContainerOptions;
use bollard::Docker;

/// 建立暫存容器執行 nvidia-smi；成功則表示 nvdocker 可用，結束後會移除容器。
pub async fn can_use_nvidia_docker(docker: &Docker, image: &str) -> bool {
    let name = format!(
        "nvidia-check-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let opts = CreateContainerOptions {
        name: Some(name.clone()),
        ..Default::default()
    };
    let create = match docker.create_container(Some(opts), check_container(image)).await {
        Ok(res) => res,
        Err(_) => return false,
    };
    let id = create.id.as_str();
    let start = docker.start_container(id, None).await;
    let _ = docker.remove_container(id, None).await;
    start.is_ok()
}

/// 檢查用的容器：在 `image` 裡用 nvidia runtime 跑 nvidia-smi，結束就自動刪掉。
fn check_container(image: &str) -> ContainerCreateBody {
    ContainerCreateBody {
        image: Some(image.to_string()),
        cmd: Some(vec!["nvidia-smi".to_string()]),
        host_config: Some(bollard::models::HostConfig {
            runtime: Some("nvidia".to_string()),
            auto_remove: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::check_container;

    #[test]
    fn runs_nvidia_smi_in_the_configured_image() {
        let body = check_container("example/cuda:12-base");
        assert_eq!(body.image.as_deref(), Some("example/cuda:12-base"));
        assert_eq!(body.cmd, Some(vec!["nvidia-smi".to_string()]));
        let host = body.host_config.expect("host config");
        assert_eq!(host.runtime.as_deref(), Some("nvidia"));
        assert_eq!(host.auto_remove, Some(true));
    }
}
