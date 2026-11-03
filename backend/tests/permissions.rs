//! 權限規則測試：每個人只能看到、操作自己的容器（管理員全部都可以），
//! 只有管理員能建立 privileged 容器、管理帳號，停用的帳號立刻失效。
//!
//! 用真的 Docker：把 alpine 標成一個只給這次測試用的 GUI 映像名稱，建幾個帶 owner label 的容器。
//! 需要 /var/run/docker.sock（CI 和本機的 Docker 都有）。不需要 Redis：被拒絕的請求在進佇列前就擋下了。

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{CreateContainerOptions, CreateImageOptions, RemoveContainerOptions, RemoveImageOptions, TagImageOptions};
use dev_dock_manager_api::config::Config;
use dev_dock_manager_api::docker::OWNER_LABEL;
use dev_dock_manager_api::{app, connect_db, db, docker, jwt, AppState};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tower::ServiceExt;

const BASE_IMAGE: &str = "alpine:3.20";
const SECRET: &str = "permissions-test-secret";

struct Fixture {
    state: AppState,
    image_name: String,
    containers: Vec<String>,
    admin: (i64, String),
    alice: (i64, String),
    bob: (i64, String),
}

async fn add_user(state: &AppState, name: &str, is_admin: bool) -> (i64, String) {
    let hash = db::hash_password("password-123").unwrap();
    let id = db::create_user(&state.pool, name, &hash, None, is_admin).await.unwrap();
    let (access, _) = jwt::issue_tokens(id, name, SECRET.as_bytes(), 3600, 3600).unwrap();
    (id, access)
}

impl Fixture {
    async fn new() -> Self {
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let image_name = format!("dev-dock-test-gui-{}", &tag[..12]);
        let config = Config {
            bind_addr: "127.0.0.1:0".into(),
            database_url: format!("sqlite://{}", std::env::temp_dir().join(format!("{image_name}.sqlite3")).display()),
            redis_url: "redis://127.0.0.1:1".into(),
            jwt_secret: SECRET.into(),
            docker_network: "bridge".into(),
            host_for_port_check: "127.0.0.1".into(),
            docker_image_name: image_name.clone(),
        };
        let pool = connect_db(&config.database_url).await.unwrap();
        let docker = docker::connect().expect("these tests need the Docker socket");
        let (repo, version) = BASE_IMAGE.split_once(':').unwrap();
        let pull = CreateImageOptions { from_image: Some(repo.into()), tag: Some(version.into()), ..Default::default() };
        let mut stream = docker.create_image(Some(pull), None, None);
        while let Some(step) = stream.next().await {
            step.expect("pull the base image");
        }
        let tag_opts = TagImageOptions { repo: Some(image_name.clone()), tag: Some("latest".into()) };
        docker.tag_image(BASE_IMAGE, Some(tag_opts)).await.unwrap();
        let state = AppState::new(config, pool, docker);
        let admin = add_user(&state, "admin", true).await;
        let alice = add_user(&state, "alice", false).await;
        let bob = add_user(&state, "bob", false).await;
        Fixture { state, image_name, containers: vec![], admin, alice, bob }
    }

    /// 建一個在跑的容器；`owner` 是 None 就是沒有主人的舊容器。
    async fn container(&mut self, image: &str, owner: Option<i64>) -> String {
        let name = format!("{}-{}", self.image_name, self.containers.len());
        let labels = owner.map(|id| HashMap::from([(OWNER_LABEL.to_string(), id.to_string())]));
        let body = ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(vec!["sleep".into(), "300".into()]),
            labels,
            ..Default::default()
        };
        let opts = CreateContainerOptions { name: Some(name.clone()), ..Default::default() };
        let id = self.state.docker.create_container(Some(opts), body).await.unwrap().id;
        self.state.docker.start_container(&id, None).await.unwrap();
        self.containers.push(id.clone());
        id
    }

    async fn call(&self, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let res = app(self.state.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn visible_ids(&self, token: &str) -> Vec<String> {
        let (status, body) = self.call("GET", "/dashboard/api/containers", token, None).await;
        assert_eq!(status, StatusCode::OK);
        let mut ids: Vec<String> = body["containers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .filter(|id| self.containers.contains(id))
            .collect();
        ids.sort();
        ids
    }

    async fn cleanup(self) {
        for id in &self.containers {
            let force = RemoveContainerOptions { force: true, ..Default::default() };
            let _ = self.state.docker.remove_container(id, Some(force)).await;
        }
        let _ = self.state.docker.remove_image(&format!("{}:latest", self.image_name), None::<RemoveImageOptions>, None).await;
        let _ = std::fs::remove_file(self.state.config.database_url.trim_start_matches("sqlite://"));
    }
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[tokio::test]
async fn users_only_see_and_control_their_own_containers() {
    let mut f = Fixture::new().await;
    let image = format!("{}:latest", f.image_name);
    let alices = f.container(&image, Some(f.alice.0)).await;
    let bobs = f.container(&image, Some(f.bob.0)).await;
    let ownerless = f.container(&image, None).await;
    let not_gui = f.container(BASE_IMAGE, Some(f.alice.0)).await;

    assert_eq!(f.visible_ids(&f.alice.1).await, vec![alices.clone()]);
    assert_eq!(f.visible_ids(&f.bob.1).await, vec![bobs.clone()]);
    // 管理員看得到全部的 GUI 容器（包括沒有主人的舊容器），但看不到別的映像建的容器
    assert_eq!(f.visible_ids(&f.admin.1).await, sorted(vec![alices.clone(), bobs.clone(), ownerless.clone()]));

    // 列表上標出主人
    let (_, body) = f.call("GET", "/dashboard/api/containers", &f.admin.1, None).await;
    let owner_of = |id: &str| body["containers"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap()["owner"].clone();
    assert_eq!(owner_of(&alices), json!("alice"));
    assert_eq!(owner_of(&ownerless), Value::Null);

    // 別人的容器、沒有主人的容器、不是 GUI 映像的容器：一律 404，不透露存在
    for (token, id) in [(&f.bob.1, &alices), (&f.alice.1, &bobs), (&f.alice.1, &ownerless), (&f.alice.1, &not_gui), (&f.admin.1, &not_gui)] {
        let (status, _) = f.call("GET", &format!("/dashboard/api/console/shell/{id}"), token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "console meta for {id}");
        let control = json!({ "cmd": "stop", "id": id });
        let (status, _) = f.call("POST", "/dashboard/api/containers/control", token, Some(control)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "control {id}");
    }
    // 自己的和管理員可以
    let (status, meta) = f.call("GET", &format!("/dashboard/api/console/shell/{alices}"), &f.alice.1, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(meta["id"], json!(alices));
    let (status, _) = f.call("GET", &format!("/dashboard/api/console/shell/{alices}"), &f.admin.1, None).await;
    assert_eq!(status, StatusCode::OK);

    f.cleanup().await;
}

#[tokio::test]
async fn only_admins_create_privileged_containers() {
    let f = Fixture::new().await;
    let body = json!({
        "container_name": "privileged-try", "ssh": "1", "user": "u", "password": "p",
        "vnc_password": "v", "root_password": "r", "privileged": true
    });
    let (status, err) = f.call("POST", "/dashboard/api/container/new", &f.alice.1, Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(err["error"], json!("Only admins can create privileged containers"));
    f.cleanup().await;
}

#[tokio::test]
async fn user_management_is_admin_only_and_takes_effect_at_once() {
    let f = Fixture::new().await;
    // 一般使用者不能列出或新增帳號
    assert_eq!(f.call("GET", "/dashboard/api/users", &f.alice.1, None).await.0, StatusCode::FORBIDDEN);
    let new_user = json!({ "username": "carol", "password": "carol-pass-1", "is_admin": false });
    assert_eq!(f.call("POST", "/dashboard/api/users", &f.alice.1, Some(new_user.clone())).await.0, StatusCode::FORBIDDEN);

    // 管理員新增帳號；密碼太短、帳號重複都會被擋
    let short = json!({ "username": "dave", "password": "short" });
    assert_eq!(f.call("POST", "/dashboard/api/users", &f.admin.1, Some(short)).await.0, StatusCode::BAD_REQUEST);
    let (status, carol) = f.call("POST", "/dashboard/api/users", &f.admin.1, Some(new_user.clone())).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(carol["is_admin"], json!(false));
    assert_eq!(f.call("POST", "/dashboard/api/users", &f.admin.1, Some(new_user)).await.0, StatusCode::BAD_REQUEST);
    let login = |password: &str| json!({ "username": "carol", "password": password });
    assert_eq!(f.call("POST", "/api/auth/token", "", Some(login("carol-pass-1"))).await.0, StatusCode::OK);

    // 停用：不能登入，已經拿到的 token 也立刻失效
    let (_, alice_me) = f.call("GET", "/dashboard/api/me", &f.alice.1, None).await;
    assert_eq!(alice_me["username"], json!("alice"));
    let disable = json!({ "is_active": false });
    let (status, alice) = f.call("PATCH", &format!("/dashboard/api/users/{}", f.alice.0), &f.admin.1, Some(disable)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(alice["is_active"], json!(false));
    assert_eq!(f.call("GET", "/dashboard/api/me", &f.alice.1, None).await.0, StatusCode::UNAUTHORIZED);
    let alice_login = json!({ "username": "alice", "password": "password-123" });
    assert_eq!(f.call("POST", "/api/auth/token", "", Some(alice_login)).await.0, StatusCode::UNAUTHORIZED);

    // 升為管理員後立刻能管理帳號
    let promote = json!({ "is_admin": true });
    f.call("PATCH", &format!("/dashboard/api/users/{}", f.bob.0), &f.admin.1, Some(promote)).await;
    assert_eq!(f.call("GET", "/dashboard/api/users", &f.bob.1, None).await.0, StatusCode::OK);

    // 管理員不能拿掉自己的角色或停用自己
    for change in [json!({ "is_admin": false }), json!({ "is_active": false })] {
        let (status, _) = f.call("PATCH", &format!("/dashboard/api/users/{}", f.admin.0), &f.admin.1, Some(change)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // 管理員重設密碼
    let reset = json!({ "new_password": "carol-pass-2" });
    let carol_id = carol["id"].as_i64().unwrap();
    assert_eq!(f.call("POST", &format!("/dashboard/api/users/{carol_id}/password"), &f.admin.1, Some(reset)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(f.call("POST", "/api/auth/token", "", Some(login("carol-pass-1"))).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(f.call("POST", "/api/auth/token", "", Some(login("carol-pass-2"))).await.0, StatusCode::OK);
    f.cleanup().await;
}

#[tokio::test]
async fn users_change_their_own_password() {
    let f = Fixture::new().await;
    let wrong = json!({ "current_password": "nope", "new_password": "new-password-1" });
    assert_eq!(f.call("POST", "/dashboard/api/me/password", &f.bob.1, Some(wrong)).await.0, StatusCode::BAD_REQUEST);
    let right = json!({ "current_password": "password-123", "new_password": "new-password-1" });
    assert_eq!(f.call("POST", "/dashboard/api/me/password", &f.bob.1, Some(right)).await.0, StatusCode::NO_CONTENT);
    let old = json!({ "username": "bob", "password": "password-123" });
    let new = json!({ "username": "bob", "password": "new-password-1" });
    assert_eq!(f.call("POST", "/api/auth/token", "", Some(old)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(f.call("POST", "/api/auth/token", "", Some(new)).await.0, StatusCode::OK);
    f.cleanup().await;
}

type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 下一則文字訊息；2 秒內沒有就是 None。
async fn next_message(ws: &mut WsStream) -> Option<String> {
    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await.ok()??.ok()?;
    msg.into_text().ok()
}

/// 開一個真的伺服器，讓 WebSocket 測試連線。
async fn serve(state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app(state)).await.unwrap() });
    format!("ws://{addr}")
}

/// 用 console WebSocket 跑一行指令，回傳輸出（或關閉的原因）。
async fn console_output(base: &str, token: &str, container: &str) -> String {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("{base}/ws/console/?container={container}")).await.unwrap();
    let first = json!({ "token": token, "action": "shell", "payload": { "Id": container } });
    ws.send(first.to_string().into()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let _ = ws.send(json!({ "action": "pty_input", "payload": { "input": "echo console-$((6*7))\n" } }).to_string().into()).await;
    let mut out = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, ws.next()).await {
        match msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                out.push_str(&t);
                if out.contains("console-42") {
                    break;
                }
            }
            tokio_tungstenite::tungstenite::Message::Close(frame) => {
                out.push_str(&format!("closed: {}", frame.map(|f| f.reason.to_string()).unwrap_or_default()));
                break;
            }
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn console_websocket_checks_the_owner() {
    let mut f = Fixture::new().await;
    let image = format!("{}:latest", f.image_name);
    let alices = f.container(&image, Some(f.alice.0)).await;
    let base = serve(f.state.clone()).await;

    // alpine has no bash, so the shell exec fails after the check; only the check matters here:
    // bob is turned away before anything runs in the container
    let bob = console_output(&base, &f.bob.1, &alices).await;
    assert!(bob.contains("closed: container not found"), "bob got: {bob}");
    let alice = console_output(&base, &f.alice.1, &alices).await;
    assert!(!alice.contains("container not found"), "alice got: {alice}");
    let nobody = console_output(&base, "not-a-token", &alices).await;
    assert!(nobody.contains("closed: Unauthorized"), "no token got: {nobody}");
    f.cleanup().await;
}

#[tokio::test]
async fn notifications_only_reach_the_owner_and_admins() {
    let mut f = Fixture::new().await;
    let image = format!("{}:latest", f.image_name);
    let alices = f.container(&image, Some(f.alice.0)).await;
    let base = serve(f.state.clone()).await;

    async fn listen(base: &str, token: &str) -> WsStream {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("{base}/ws/notifications/")).await.unwrap();
        ws.send(json!({ "token": token }).to_string().into()).await.unwrap();
        ws
    }
    let mut to_alice = listen(&base, &f.alice.1).await;
    let mut to_bob = listen(&base, &f.bob.1).await;
    let mut to_admin = listen(&base, &f.admin.1).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // alice stops her container: the WAITING notice is sent before the job is queued
    // (queueing then fails because there is no Redis in this test, which doesn't matter here)
    f.call("POST", "/dashboard/api/containers/control", &f.alice.1, Some(json!({ "cmd": "stop", "id": alices }))).await;

    let got_alice = next_message(&mut to_alice).await;
    let got_admin = next_message(&mut to_admin).await;
    let got_bob = next_message(&mut to_bob).await;
    assert!(got_alice.is_some_and(|m| m.contains("WAITING")));
    assert!(got_admin.is_some_and(|m| m.contains("WAITING")));
    assert!(got_bob.is_none(), "bob must not hear about alice's container");
    f.cleanup().await;
}
