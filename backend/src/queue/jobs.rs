//! 佇列 worker：以 BRPOP 取任務，依類型呼叫 bollard（建立/啟動/停止/刪除/重啟容器），
//! 完成後透過 broadcast 發送通知給 WebSocket 客戶端。

use super::{EnqueuedJob, Job, QUEUE_KEY};
use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use bollard::query_parameters::{
    CreateContainerOptions, RemoveContainerOptions, StopContainerOptions,
};
use bollard::Docker;
use std::collections::HashMap;
use std::time::Duration;

/// 常駐迴圈：連 Redis 與 Docker，BRPOP 取 job、執行 run_job、將結果經 notify_tx 廣播。
pub async fn run_worker(
    redis_url: String,
    docker_network: String,
    notify_tx: tokio::sync::broadcast::Sender<String>,
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
        let mut conn = match client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Worker: Redis connection lost: {}; reconnecting...", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let result: Result<(String, String), _> =
            redis::cmd("BRPOP").arg(QUEUE_KEY).arg(5).query_async(&mut conn).await;
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
        let task_id = enqueued.task_id;
        match run_job(&docker, &docker_network, enqueued.job).await {
            Ok((action, details)) => {
                let msg =
                    serde_json::json!({ "message": { "action": action, "details": details } });
                let _ = notify_tx.send(msg.to_string());
            }
            Err(e) => {
                tracing::error!("Worker: job {} failed: {}", task_id, e);
            }
        }
    }
}

async fn run_job(
    docker: &Docker,
    _docker_network: &str,
    job: Job,
) -> Result<(String, String), String> {
    match job {
        Job::RunImage {
            image_name,
            ssh_port,
            name,
            user,
            password,
            vnc_password,
            root_password,
            privileged,
            nvdocker,
            docker_network: net,
        } => run_image(
            docker,
            &net,
            &image_name,
            ssh_port,
            &name,
            &user,
            &password,
            &vnc_password,
            &root_password,
            privileged,
            nvdocker,
        )
        .await,
        Job::StartContainer { id } => run_start(docker, &id).await,
        Job::StopContainer { id } => run_stop(docker, &id).await,
        Job::RemoveContainer { id } => run_remove(docker, &id).await,
        Job::RestartContainer { id } => run_restart(docker, &id).await,
    }
}

async fn run_image(
    docker: &Docker,
    docker_network: &str,
    image_name: &str,
    ssh_port: u16,
    name: &str,
    user: &str,
    password: &str,
    vnc_password: &str,
    root_password: &str,
    privileged: bool,
    nvdocker: bool,
) -> Result<(String, String), String> {
    let mut port_bindings = HashMap::new();
    port_bindings.insert(
        "22/tcp".to_string(),
        Some(vec![PortBinding {
            host_ip: None,
            host_port: Some(ssh_port.to_string()),
        }]),
    );
    let mut binds = Vec::new();
    if crate::docker::is_linux() {
        binds.push("/etc/localtime:/etc/localtime:ro".to_string());
    }
    let mut labels = HashMap::new();
    labels.insert("traefik.enable".to_string(), "true".to_string());
    labels.insert(
        format!("traefik.http.routers.d-gui-{}.rule", name),
        format!("PathPrefix(`/novnc/{}/`)", name),
    );
    labels.insert(
        format!("traefik.http.services.d-gui-{}.loadbalancer.server.port", name),
        "6901".to_string(),
    );
    labels.insert(
        format!(
            "traefik.http.middlewares.d-gui-{}-strip-prefix.stripprefix.prefixes",
            name
        ),
        format!("/novnc/{}/", name),
    );
    labels.insert(
        format!("traefik.http.routers.d-gui-{}.middlewares", name),
        format!("d-gui-{}-strip-prefix", name),
    );
    labels.insert("traefik.docker.network".to_string(), docker_network.to_string());

    let mut device_requests = Vec::new();
    if nvdocker {
        device_requests.push(bollard::models::DeviceRequest {
            driver: Some("nvidia".to_string()),
            count: Some(-1),
            device_ids: None,
            capabilities: Some(vec![vec!["gpu".to_string()]]),
            options: None,
        });
    }

    let env = vec![
        format!("VNC_PW={}", vnc_password),
        "VNC_RESOLUTION=1600x900".to_string(),
        format!("DEFAULT_USER={}", user),
        format!("DEFAULT_USER_PASSWORD={}", password),
        format!("ROOT_PASSWORD={}", root_password),
    ];

    let host_config = HostConfig {
        port_bindings: Some(port_bindings),
        binds: if binds.is_empty() {
            None
        } else {
            Some(binds)
        },
        privileged: Some(privileged),
        device_requests: if device_requests.is_empty() {
            None
        } else {
            Some(device_requests)
        },
        ..Default::default()
    };

    let config = ContainerCreateBody {
        image: Some(image_name.to_string()),
        host_config: Some(host_config),
        env: Some(env),
        labels: Some(labels),
        ..Default::default()
    };

    let opts = CreateContainerOptions {
        name: Some(name.to_string()),
        ..Default::default()
    };
    let create = docker
        .create_container(Some(opts), config)
        .await
        .map_err(|e| e.to_string())?;
    let id = create.id.as_str();
    docker
        .start_container(id, None)
        .await
        .map_err(|e| e.to_string())?;

    // Connect container to network (bollard 0.20: NetworkConnectRequest in models)
    let connect_body = bollard::models::NetworkConnectRequest {
        container: id.to_string(),
        endpoint_config: None,
    };
    if let Err(e) = docker
        .connect_network(docker_network, connect_body)
        .await
    {
        tracing::warn!(
            "Container [{}] created but connect to network {} failed: {}",
            name,
            docker_network,
            e
        );
    }

    let msg = format!("Container [{}] ({}) has been created", name, image_name);
    Ok(("CREATED".to_string(), msg))
}

async fn run_start(docker: &Docker, id: &str) -> Result<(String, String), String> {
    docker
        .start_container(id, None)
        .await
        .map_err(|e| e.to_string())?;
    let inspect = docker.inspect_container(id, None).await.map_err(|e| e.to_string())?;
    let name = inspect.name.as_deref().unwrap_or(id).trim_start_matches('/');
    Ok(("STARTED".to_string(), format!("Container [{}] has been started", name)))
}

async fn run_stop(docker: &Docker, id: &str) -> Result<(String, String), String> {
    docker
        .stop_container(id, None::<StopContainerOptions>)
        .await
        .map_err(|e| e.to_string())?;
    let inspect = docker.inspect_container(id, None).await.map_err(|e| e.to_string())?;
    let name = inspect.name.as_deref().unwrap_or(id).trim_start_matches('/');
    Ok(("STOPPED".to_string(), format!("Container [{}] has been stopped", name)))
}

async fn run_remove(docker: &Docker, id: &str) -> Result<(String, String), String> {
    let inspect = docker.inspect_container(id, None).await.map_err(|e| e.to_string())?;
    let name = inspect.name.as_deref().unwrap_or(id).trim_start_matches('/').to_string();
    docker
        .remove_container(id, None::<RemoveContainerOptions>)
        .await
        .map_err(|e| e.to_string())?;
    Ok(("REMOVED".to_string(), format!("Container [{}] has been removed", name)))
}

async fn run_restart(docker: &Docker, id: &str) -> Result<(String, String), String> {
    docker
        .restart_container(id, None)
        .await
        .map_err(|e| e.to_string())?;
    let inspect = docker.inspect_container(id, None).await.map_err(|e| e.to_string())?;
    let name = inspect.name.as_deref().unwrap_or(id).trim_start_matches('/');
    Ok(("RESTARTED".to_string(), format!("Container [{}] has been restarted", name)))
}