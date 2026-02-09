//! Dev-dock-manager Rust API 程式進入點。
//!
//! ；可綁定不同 port（如 8000）或由反向代理分流。
//!
//! 子指令（對應 Django 建立帳號）：
//!   create-user <username> <password> [--email EMAIL] [--staff]
//!   或透過環境變數：CREATE_USER_USERNAME, CREATE_USER_PASSWORD, CREATE_USER_EMAIL, CREATE_USER_STAFF=1

use dev_dock_manager_api::{create_user_cli, run};

fn parse_create_user_args() -> Option<(String, String, Option<String>, bool)> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) != Some("create-user") {
        return None;
    }
    let username = std::env::var("CREATE_USER_USERNAME").ok().or_else(|| args.get(2).cloned());
    let password = std::env::var("CREATE_USER_PASSWORD").ok().or_else(|| args.get(3).cloned());
    if username.is_none() || password.is_none() {
        let program = args.first().map(|s| s.as_str()).unwrap_or("dev-dock-manager-api");
        eprintln!("Usage: {} create-user <username> <password> [--email EMAIL] [--staff]", program);
        eprintln!("   or set env: CREATE_USER_USERNAME, CREATE_USER_PASSWORD, CREATE_USER_EMAIL (optional), CREATE_USER_STAFF=1 (optional)");
        std::process::exit(1);
    }
    let email = std::env::var("CREATE_USER_EMAIL").ok().or_else(|| {
        let mut i = 4;
        while i + 1 < args.len() {
            if args[i] == "--email" {
                return Some(args[i + 1].clone());
            }
            i += 1;
        }
        None
    });
    let is_staff = std::env::var("CREATE_USER_STAFF").map(|s| s == "1" || s.eq_ignore_ascii_case("true")).unwrap_or(false)
        || args[4..].iter().any(|a| a == "--staff");
    Some((username.unwrap(), password.unwrap(), email, is_staff))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some((username, password, email, is_staff)) = parse_create_user_args() {
        create_user_cli(username, password, email, is_staff).await
    } else {
        run().await
    }
}
