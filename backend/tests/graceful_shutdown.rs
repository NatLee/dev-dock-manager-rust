//! 終止訊號測試：後台收到 SIGTERM / SIGINT 時要自己正常結束（結束碼 0），
//! 而不是被訊號直接殺掉，或在容器裡（PID 1）完全不理會、等到 SIGKILL。
//!
//! 啟動真正的執行檔，等 `/health` 回 200，送訊號，確認幾秒內以結束碼 0 離開。
//! 需要 Docker socket（後台啟動時會連 Docker）；Redis 指到沒人聽的 port，worker 只會一直重連。
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind a free port")
        .local_addr()
        .unwrap()
        .port()
}

fn health_ok(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response.starts_with("HTTP/1.1 200")
}

fn start_api(port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_dev-dock-manager-api"))
        .env("BIND_ADDR", format!("127.0.0.1:{port}"))
        .env("DATABASE_URL", "sqlite::memory:")
        .env("REDIS_URL", "redis://127.0.0.1:1")
        .env("JWT_SECRET", "graceful-shutdown-test")
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start dev-dock-manager-api")
}

fn assert_exits_cleanly_on(signal: &str) {
    let port = free_port();
    let mut child = start_api(port);

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while !health_ok(port) {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("API exited before it was ready: {status}");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("API did not answer /health within {STARTUP_TIMEOUT:?}");
        }
        sleep(Duration::from_millis(100));
    }

    let sent = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(child.id().to_string())
        .status()
        .expect("run kill");
    assert!(sent.success(), "kill -{signal} failed");

    let deadline = Instant::now() + EXIT_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "expected a clean exit after SIG{signal}, got {status}"
            );
            return;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("API was still running {EXIT_TIMEOUT:?} after SIG{signal}");
        }
        sleep(Duration::from_millis(50));
    }
}

#[test]
fn exits_cleanly_on_sigterm() {
    assert_exits_cleanly_on("TERM");
}

#[test]
fn exits_cleanly_on_sigint() {
    assert_exits_cleanly_on("INT");
}
