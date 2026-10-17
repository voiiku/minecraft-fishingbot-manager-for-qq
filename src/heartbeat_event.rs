//! OneBot 心跳事件：把协议端上报的 `meta_event.heartbeat` 认出来，交给库里的心跳看门狗。
//!
//! kovi-onebot 只导出了 `lifecycle` 一类元事件、没有心跳，所以本项目自己实现一个事件类型：
//! kovi 会把驱动收到的事件交给**每个**已注册事件类型的 `Event::de`，在 `de` 里认心跳载荷即可，
//! 不必向协议端查询状态（也不占 API 配额）。
//!
//! 与 `command_router`（认指令、洗参数、分发）分开放：心跳与指令是两条互不相干的链路，
//! 本文件只负责**解析载荷**，判定窗口与收工逻辑都在库里的 `heartbeat` 模块
//! （见 [docs/heartbeat.md](../docs/heartbeat.md)）。

use kovi::bot::BotInformation;
use kovi::event::{Event, InternalEvent};
use kovi::types::ApiAndOptOneshot;
use mc_qq_fishbot::HeartbeatReport;

/// OneBot 心跳事件（`post_type = "meta_event"` 且 `meta_event_type = "heartbeat"`）。
///
/// 非心跳载荷一律返回 `None` 交给框架忽略；心跳载荷里额外取出 `status.online`
/// （账号是否在线）交给看门狗。
pub struct HeartbeatEvent {
    /// 这次心跳里能取到的信息。
    pub report: HeartbeatReport,
}

impl Event for HeartbeatEvent {
    fn de(
        event: &InternalEvent,
        _: &BotInformation,
        _: &tokio::sync::mpsc::Sender<ApiAndOptOneshot>,
    ) -> Option<Self> {
        let InternalEvent::DriverEvent(json) = event else {
            return None;
        };

        heartbeat_report(json).map(|report| Self { report })
    }
}

/// 解析心跳载荷：是心跳就返回它能提供的信息，否则返回 `None`。
///
/// 只按 `post_type` 与 `meta_event_type` 判断：其它元事件（如 `lifecycle`）与普通消息都必须
/// 返回 `None`，否则会误导看门狗；`status` 里没有 `online`、`online` 类型不符，
/// 一律当成"没上报"，不影响心跳识别本身。
pub fn heartbeat_report(payload: &kovi::serde_json::Value) -> Option<HeartbeatReport> {
    let is_heartbeat = payload.get("post_type").and_then(|value| value.as_str())
        == Some("meta_event")
        && payload
            .get("meta_event_type")
            .and_then(|value| value.as_str())
            == Some("heartbeat");
    if !is_heartbeat {
        return None;
    }

    Some(HeartbeatReport {
        online: payload
            .get("status")
            .and_then(|status| status.get("online"))
            .and_then(|value| value.as_bool()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kovi::serde_json::json;

    /// 协议端（NapCat）上报的心跳载荷要能被认出来，并取出看门狗要用的账号在线状态。
    #[test]
    fn heartbeat_payload_is_recognized() {
        let payload = json!({
            "post_type": "meta_event",
            "meta_event_type": "heartbeat",
            "time": 1758888888u32,
            "self_id": 123456789i64,
            "status": { "online": true, "good": true },
            "interval": 30000u32
        });

        assert_eq!(
            heartbeat_report(&payload),
            Some(HeartbeatReport { online: Some(true) })
        );
    }

    /// 心跳载荷里缺 `status`（NapCat 的 `status.online` 可能是 `undefined`）、`online`
    /// 不是布尔时仍要认出来，只是这一项为 `None`。
    #[test]
    fn heartbeat_without_online_field_is_still_recognized() {
        let cases = [
            json!({ "post_type": "meta_event", "meta_event_type": "heartbeat" }),
            json!({
                "post_type": "meta_event",
                "meta_event_type": "heartbeat",
                "status": { "online": "true", "good": true }
            }),
        ];

        for payload in cases {
            assert_eq!(
                heartbeat_report(&payload),
                Some(HeartbeatReport::default()),
                "{payload}"
            );
        }
    }

    /// 其它元事件（`lifecycle` 等）与普通消息都不能当成心跳，否则会误导看门狗。
    #[test]
    fn other_payloads_are_not_heartbeats() {
        let payloads = [
            json!({
                "post_type": "meta_event",
                "meta_event_type": "lifecycle",
                "sub_type": "connect"
            }),
            json!({ "post_type": "message", "message_type": "group", "raw_message": "hello" }),
            json!({ "post_type": "meta_event" }),
            json!({ "post_type": "meta_event", "meta_event_type": 5000u32 }),
        ];

        for payload in payloads {
            assert_eq!(heartbeat_report(&payload), None, "{payload} 不该被当成心跳");
        }
    }
}
