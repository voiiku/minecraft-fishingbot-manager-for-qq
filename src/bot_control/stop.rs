//! `/stop`：停止单个机器人。

use crate::utils::{BotDirEntry, StartupQueue, is_bot_screen_running, stop_one_bot_session};

/// 停止单个机器人（`/stop`）。
///
/// 校验都在本函数内做（权限在业务层，不在路由层）：[`BotDirEntry::find_for_caller`] 判定"目录存在 +
/// 调用者有权限"（口径＝目录归属，管理员可跨目录），会话未运行时直接返回"无需停止"。
/// 停止流程与 `/stopall` 共用 [`stop_one_bot_session`]。
/// 停止流程会立即复核会话是否已消失（见 `utils::stop_sessions`），因此可以返回"已成功停止"。
pub fn stop_bot(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    // 目录存在 + 调用者有权限，两者都由这一句保证（口径＝目录归属）。
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    // 还在队列里等待：撤单就是最彻底的「停止」，不必等到会话出现。
    if StartupQueue::remove_waiting_bot(&bot_dir_entry.bot_name) {
        log::info!("机器人 {raw_bot_name} 已取消排队启动");
        return Ok(format!("已取消机器人 '{raw_bot_name}' 的排队启动。"));
    }

    // 正在启动或复核的条目无法撤单；会话刚出现时也不应停止，应提示用户稍后重试。
    if StartupQueue::bot_in_queue(&bot_dir_entry.bot_name) {
        return Err(format!(
            "机器人 '{raw_bot_name}' 正在启动，暂时无法停止；请稍后重试，或使用 /stopall。"
        ));
    }

    // 没有运行中的会话时直接告知用户，无需进入停止流程。
    if !is_bot_screen_running(&bot_dir_entry.bot_name) {
        return Err(format!("机器人 '{raw_bot_name}' 未运行，无需停止。"));
    }

    let report = stop_one_bot_session(bot_dir_entry);
    if !report.is_ok() {
        return Err(report.failure_text());
    }

    log::info!("机器人 {raw_bot_name} 已停止");
    Ok(format!("已停止机器人 '{raw_bot_name}'。"))
}
