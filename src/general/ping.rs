//! `/ping`：探测游戏服务器连通性与在线人数。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
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

    let target_addresses =
        match tokio::time::timeout(timeout, tokio::net::lookup_host((target_host, target_port)))
            .await
        {
            Ok(Ok(addresses)) => addresses.collect::<Vec<_>>(),
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

    // 默认目标由运维配置；用户指定的目标只允许公网地址，并复用解析结果以避免 DNS 重绑定。
    if host.is_some()
        && target_addresses
            .iter()
            .any(|address| is_local_target_ip(address.ip()))
    {
        return Err(
            "自定义探测目标必须为公网地址；内网服务器请通过 [ping] 配置默认目标。".to_string(),
        );
    }

    let mut stream = None;
    let mut last_error = "未解析到服务器地址".to_string();
    for address in target_addresses {
        match tokio::time::timeout(
            timeout.saturating_sub(start.elapsed()),
            TcpStream::connect(address),
        )
        .await
        {
            Ok(Ok(connected)) => {
                stream = Some(connected);
                break;
            }
            Ok(Err(error)) => last_error = error.to_string(),
            Err(_) => {
                return Err(format!(
                    "连接服务器 [{target_host}:{target_port}] 超时（{} 毫秒无响应）。",
                    ping_config.timeout_ms
                ));
            }
        }
    }
    let Some(mut stream) = stream else {
        return Err(format!(
            "连接服务器 [{target_host}:{target_port}] 失败：\n{last_error}"
        ));
    };

    match tokio::time::timeout(
        timeout.saturating_sub(start.elapsed()),
        craftping_ping(&mut stream, target_host, target_port),
    )
    .await
    {
        Ok(Ok(response)) => {
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
        Ok(Err(e)) => Err(format!(
            "服务器 [{target_host}:{target_port}] 协议解析失败：\n{e}"
        )),
        Err(_) => Err(format!(
            "探测服务器 [{target_host}:{target_port}] 超时（{} 毫秒无响应）。",
            ping_config.timeout_ms
        )),
    }
}

fn is_local_target_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_local_ipv4(ip),
        IpAddr::V6(ip) => is_local_ipv6(ip) || ip.to_ipv4_mapped().is_some_and(is_local_ipv4),
    }
}

fn is_local_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
        || octets[0] >= 240
}

fn is_local_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_multicast()
        || (ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_reserved_targets_are_rejected() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_local_target_ip(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn public_targets_are_allowed() {
        for address in ["8.8.8.8", "1.1.1.1", "2001:4860:4860::8888"] {
            assert!(!is_local_target_ip(address.parse().unwrap()), "{address}");
        }
    }
}
