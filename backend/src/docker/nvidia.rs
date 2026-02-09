//! 檢測本機是否可用 NVIDIA Docker（nvidia runtime）。
//! 透過建立並執行一個最小 nvidia/cuda 容器執行 nvidia-smi 判斷。

use bollard::models::ContainerCreateBody;
use bollard::query_parameters::CreateContainerOptions;
use bollard::Docker;

/// 建立暫存容器執行 nvidia-smi；成功則表示 nvdocker 可用，結束後會移除容器。
pub async fn can_use_nvidia_docker(docker: &Docker) -> bool {
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
    let body = ContainerCreateBody {
        image: Some("nvidia/cuda:11.0.3-base-ubuntu20.04".to_string()),
        cmd: Some(vec!["nvidia-smi".to_string()]),
        host_config: Some(bollard::models::HostConfig {
            runtime: Some("nvidia".to_string()),
            auto_remove: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let create = match docker.create_container(Some(opts), body).await {
        Ok(res) => res,
        Err(_) => return false,
    };
    let id = create.id.as_str();
    let start = docker.start_container(id, None).await;
    let _ = docker.remove_container(id, None).await;
    start.is_ok()
}
