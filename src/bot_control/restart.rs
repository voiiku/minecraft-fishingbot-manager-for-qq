//! `/restart`：重启单个机器人。

use crate::utils::{BotDirEntry, StartupQueue, restart_bot_sessions};

/// 重启单个机器人（`/restart`）。
///
/// 校验都在本函数内做（权限在业务层，不在路由层）：只定位目录，[`BotDirEntry::find_for_caller`]
/// 同时判定"目录存在 + 调用者有权限"（口径＝目录归属，管理员可跨目录）。
/// 重启流程与 `/restartall` 共用 [`restart_bot_sessions`]。
///
/// 回复里"停止已确认"能兑现（停止流程立即复核会话消失），但启动部分与 `/start` 一样
/// 只能说"已发送"：这里的复核只能确认会话出现了。
pub fn restart_bot(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    log::info!("接收到机器人 {raw_bot_name} 的重启命令");

    // 排队中的先撤单再重启；正在启动/复核的不能撤，直接拒绝，避免两个流程抢同一个会话。
    if StartupQueue::bot_in_queue(&bot_dir_entry.bot_name)
        && !StartupQueue::remove_waiting_bot(&bot_dir_entry.bot_name)
    {
        return Err(format!("机器人 '{raw_bot_name}' 正在启动，请稍后重试。"));
    }

    // "停止 + 重新进入启动队列 + 复核"都在共用实现里。
    let report = restart_bot_sessions(&[bot_dir_entry]);
    if !report.is_ok() {
        return Err(report.failure_text());
    }
    if report.succeeded.is_empty() {
        return Err(report.skipped.join(" / "));
    }

    Ok(format!(
        "已发送机器人 '{raw_bot_name}' 的重启命令；停止已确认，请用 /status 查看是否上线。"
    ))
}
