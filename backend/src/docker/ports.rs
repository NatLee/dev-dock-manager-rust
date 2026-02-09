//! 埠解析與佔用檢查：將 Docker 埠綁定對應為服務名（vnc/novnc/ssh）與 host port，並可檢查 host 上埠是否被佔用。

use bollard::models::PortBinding;
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::time::Duration;

const PORT_SERVICE: &[(&str, &str)] = &[
    ("5901/tcp", "vnc"),
    ("6901/tcp", "novnc"),
    ("22/tcp", "ssh"),
];

/// 將 bollard 的 port_bindings 轉成「服務名 -> host port」對應（僅處理 vnc/novnc/ssh）。
pub fn parse_ports_bollard(
    port_bindings: &HashMap<String, Option<Vec<PortBinding>>>,
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for (port, bindings) in port_bindings {
        if let Some(binds) = bindings {
            if let Some(first) = binds.first() {
                if let Some(ref host_port) = first.host_port {
                    if let Some(service) = PORT_SERVICE.iter().find(|(p, _)| *p == port) {
                        result.insert((*service.1).to_string(), host_port.clone());
                    }
                }
            }
        }
    }
    result
}

/// 將泛型 port_bindings（HashMap 格式）轉成服務名 -> host port；供測試或其它呼叫端使用。
pub fn parse_ports(port_bindings: &HashMap<String, Option<Vec<HashMap<String, String>>>>) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for (port, bindings) in port_bindings {
        if let Some(binds) = bindings {
            if let Some(first) = binds.first() {
                if let Some(host_port) = first.get("HostPort") {
                    if let Some(service) = PORT_SERVICE.iter().find(|(p, _)| *p == port.as_str()) {
                        result.insert((*service.1).to_string(), host_port.clone());
                    }
                }
            }
        }
    }
    result
}

/// 以 TCP 連線嘗試判斷指定 host:port 是否已被佔用。
pub fn check_port_in_use(host: &str, port: u16) -> bool {
    let addr = match (host, port).to_socket_addrs() {
        Ok(mut a) => a.next(),
        Err(_) => return false,
    };
    let addr = match addr {
        Some(a) => a,
        None => return false,
    };
    std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok()
}
