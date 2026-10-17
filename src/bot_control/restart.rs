//! `/restart`：重启单个机器人。

use crate::utils::{BotDirEntry, LaunchTiming, restart_bot_sessions};

/// 重启单个机器人（`/restart`）。
///
/// 校验都在本函数内做（权限在业务层，不在路由层）：只定位目录，[`BotDirEntry::find_for_caller`]
/// 同时判定"目录存在 + 调用者有权限"（口径＝目录归属，管理员可跨目录）。
/// 重启流程与 `/restartall` 共用 [`restart_bot_sessions`]。
///
/// 回复里"停止已确认"能兑现（停止流程当场复核会话消失），但拉起部分与 `/start` 一样
/// 只能说"已发送" —— 见 [bot-control.md](../../docs/bot-control.md) 第二节的用词原则。
pub fn restart_bot(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    log::info!("接收到机器人 {raw_bot_name} 的重启命令");
    // 单个指令用单机等待参数（不错峰），"停止 + 拉起 + 复核"都在共用实现里。
    let report = restart_bot_sessions(&[bot_dir_entry], LaunchTiming::single());
    if !report.is_ok() {
        return Err(report.failure_text());
    }

    Ok(format!(
        "已发送机器人 '{raw_bot_name}' 的重启命令（停止已确认；启动是否上线请用 /status 确认）"
    ))
}
