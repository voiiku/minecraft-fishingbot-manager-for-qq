//! `/ping`：探测游戏服务器连通性与在线人数。

use std::time::{Duration, Instant};

use craftping::tokio::ping as craftping_ping;
use tokio::net::TcpStream;

use crate::config;
use crate::utils::Report;

/// 探测游戏服务器状态；未指定地址与端口时使用配置里的 `ping.*`。
pub async fn ping(host: Option<&str>, port: Option<u16>) -> Result<String, String> {
    let ping_config = &config::get().ping;
    let target_host = host.unwrap_or(ping_config.default_ping_host.as_str());
    let target_port = port.unwrap_or(ping_config.default_ping_port);
    let timeout = Duration::from_millis(ping_config.timeout_ms);

    let start = Instant::now();

    let connect_result =
        tokio::time::timeout(timeout, TcpStream::connect((target_host, target_port))).await;

    let mut stream = match connect_result {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(format!(
                "连接服务器 [{target_host}:{target_port}] 失败：\n{e}"
            ));
        }
        Err(_) => {
            return Err(format!(
                "连接服务器 [{target_host}:{target_port}] 超时（{} 毫秒无响应）。",
                ping_config.timeout_ms
            ));
        }
    };

    match craftping_ping(&mut stream, target_host, target_port).await {
        Ok(response) => {
            let latency = start.elapsed().as_millis();

            let mut report = Report::new(format!("MC 服务器 [{target_host}:{target_port}]"));
            report.basics.push("状态：在线".to_string());
            report.basics.push(format!(
                "人数：{}/{}",
                response.online_players, response.max_players
            ));
            report.basics.push(format!(
                "延迟：{latency} ms（钓鱼服务器到游戏服务器的延迟）"
            ));

            Ok(report.render())
        }
        Err(e) => Err(format!(
            "服务器 [{target_host}:{target_port}] 协议解析失败：\n{e}"
        )),
    }
}
