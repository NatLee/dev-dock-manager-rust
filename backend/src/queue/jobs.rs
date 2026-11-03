//! 佇列 worker：以 BRPOP 取任務，依類型呼叫 bollard（建立/啟動/停止/刪除/重啟容器），
//! 完成後把結果當成通知（帶著容器主人）廣播給 WebSocket。

use super::{EnqueuedJob, Job, NewContainer, Notification, QUEUE_KEY};
use crate::docker::{gui_container_labels, GUI_NOVNC_PORT, GUI_SSH_PORT};
use bollard::models::{ContainerCreateBody, DeviceRequest, HostConfig, PortBinding};
use bollard::query_parameters::{
    CreateContainerOptions, RemoveContainerOptions, StopContainerOptions,
};
use bollard::Docker;
use std::collections::HashMap;
use std::time::Duration;

/// 常駐迴圈：連 Redis 與 Docker，BRPOP 取 job、執行 run_job、將結果經 notify_tx 廣播。
///
/// `shutdown` 變成 true（或 sender 被丟掉）時停止：等待中的 BRPOP 或重連會中斷，
/// 但已經開始執行的 job 會做完，避免容器建到一半。
pub async fn run_worker(
    redis_url: String,
    notify_tx: tokio::sync::broadcast::Sender<Notification>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let docker = match crate::docker::connect() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Worker: Docker connect failed: {}", e);
            return;
        }
    };
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Worker: Redis connect failed: {}", e);
            return;
        }
    };
    loop {
        if *shutdown.borrow() {
            break;
        }
        let connected = tokio::select! {
            c = client.get_multiplexed_async_connection() => c,
            _ = shutdown.changed() => break,
        };
        let mut conn = match connected {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Worker: Redis connection lost: {}; reconnecting...", e);
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => continue,
                }
            }
        };
        let mut brpop = redis::cmd("BRPOP");
        brpop.arg(QUEUE_KEY).arg(5);
        // biased：BRPOP 已經拿到 job 時先處理它，不要因為同時收到停止通知就丟掉。
        let result: Result<(String, String), _> = tokio::select! {
            biased;
            r = brpop.query_async(&mut conn) => r,
            _ = shutdown.changed() => break,
        };
        let (_, payload) = match result {
            Ok(t) => t,
            Err(_) => continue,
        };
        let enqueued: EnqueuedJob = match serde_json::from_str(&payload) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Worker: invalid job payload: {}", e);
                continue;
            }
        };
        match run_job(&docker, enqueued.job).await {
            Ok((action, details)) => {
                let _ = notify_tx.send(Notification::new(enqueued.owner_id, action, &details, None));
            }
            Err(e) => {
                tracing::error!("Worker: job {} failed: {}", enqueued.task_id, e);
            }
        }
    }
    tracing::info!("Worker: stopped");
}

async fn run_job(docker: &Docker, job: Job) -> Result<(&'static str, String), String> {
    match job {
        Job::RunImage(spec) => run_image(docker, &spec).await,
        Job::StartContainer { id } => run_start(docker, &id).await,
        Job::StopContainer { id } => run_stop(docker, &id).await,
        Job::RemoveContainer { id } => run_remove(docker, &id).await,
        Job::RestartContainer { id } => run_restart(docker, &id).await,
    }
}

/// 建立並啟動一個 GUI 容器：只公開 SSH，noVNC 交給 Traefik（label），直接接在管理介面的網路上。
async fn run_image(docker: &Docker, spec: &NewContainer) -> Result<(&'static str, String), String> {
    let port_bindings = HashMap::from([(
        format!("{GUI_SSH_PORT}/tcp"),
        Some(vec![PortBinding {
            host_ip: None,
            host_port: Some(spec.ssh_port.to_string()),
        }]),
    )]);
    let binds = crate::docker::is_linux().then(|| vec!["/etc/localtime:/etc/localtime:ro".to_string()]);
    let device_requests = spec.nvdocker.then(|| {
        vec![DeviceRequest {
            driver: Some("nvidia".to_string()),
            count: Some(-1),
            capabilities: Some(vec![vec!["gpu".to_string()]]),
            ..Default::default()
        }]
    });
    // 解析度等其他設定用映像自己的預設值。
    let env = vec![
        format!("VNC_PW={}", spec.vnc_password),
        format!("DEFAULT_USER={}", spec.user),
        format!("DEFAULT_USER_PASSWORD={}", spec.password),
        format!("ROOT_PASSWORD={}", spec.root_password),
    ];
    let config = ContainerCreateBody {
        image: Some(spec.image_name.clone()),
        env: Some(env),
        labels: Some(gui_container_labels(&spec.name, &spec.docker_network, GUI_NOVNC_PORT, spec.owner_id)),
        host_config: Some(HostConfig {
            port_bindings: Some(port_bindings),
            binds,
            privileged: Some(spec.privileged),
            device_requests,
            network_mode: Some(spec.docker_network.clone()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let opts = CreateContainerOptions {
        name: Some(spec.name.clone()),
        ..Default::default()
    };
    let create = docker
        .create_container(Some(opts), config)
        .await
        .map_err(|e| e.to_string())?;
    docker
        .start_container(&create.id, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok((
        "CREATED",
        format!("Container [{}] ({}) has been created", spec.name, spec.image_name),
    ))
}

async fn container_name(docker: &Docker, id: &str) -> Result<String, String> {
    let inspect = docker.inspect_container(id, None).await.map_err(|e| e.to_string())?;
    Ok(inspect.name.as_deref().unwrap_or(id).trim_start_matches('/').to_string())
}

async fn run_start(docker: &Docker, id: &str) -> Result<(&'static str, String), String> {
    docker.start_container(id, None).await.map_err(|e| e.to_string())?;
    let name = container_name(docker, id).await?;
    Ok(("STARTED", format!("Container [{}] has been started", name)))
}

async fn run_stop(docker: &Docker, id: &str) -> Result<(&'static str, String), String> {
    docker
        .stop_container(id, None::<StopContainerOptions>)
        .await
        .map_err(|e| e.to_string())?;
    let name = container_name(docker, id).await?;
    Ok(("STOPPED", format!("Container [{}] has been stopped", name)))
}

async fn run_remove(docker: &Docker, id: &str) -> Result<(&'static str, String), String> {
    let name = container_name(docker, id).await?;
    docker
        .remove_container(id, None::<RemoveContainerOptions>)
        .await
        .map_err(|e| e.to_string())?;
    Ok(("REMOVED", format!("Container [{}] has been removed", name)))
}

async fn run_restart(docker: &Docker, id: &str) -> Result<(&'static str, String), String> {
    docker.restart_container(id, None).await.map_err(|e| e.to_string())?;
    let name = container_name(docker, id).await?;
    Ok(("RESTARTED", format!("Container [{}] has been restarted", name)))
}
