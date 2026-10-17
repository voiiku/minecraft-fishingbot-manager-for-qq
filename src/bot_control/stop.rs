//! `/stop`：停止单个机器人。

use crate::utils::{BotDirEntry, is_bot_screen_running, stop_one_bot_session};

/// 停止单个机器人（`/stop`）。
///
/// 校验都在本函数内做（权限在业务层，不在路由层）：[`BotDirEntry::find_for_caller`] 判定"目录存在 +
/// 调用者有权限"（口径＝目录归属，管理员可跨目录），会话不在跑时直接回"无需停止"。
/// 停止流程与 `/stopall` 共用 [`stop_one_bot_session`]。
/// 停止流程会当场复核会话是否真的消失（见 `utils::stop_sessions`），所以这里可以回"已成功停止"。
pub fn stop_bot(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    // 目录存在 + 调用者有权限，两者都由这一句保证（口径＝目录归属）。
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    // 没在跑就没有可停的会话，直接告诉用户，不用走停止流程。
    if !is_bot_screen_running(&bot_dir_entry.bot_name) {
        return Err(format!("机器人 '{raw_bot_name}' 未运行，无需停止！"));
    }

    let report = stop_one_bot_session(bot_dir_entry);
    if !report.is_ok() {
        return Err(report.failure_text());
    }

    log::info!("机器人 {raw_bot_name} 已停止");
    Ok(format!("已成功停止机器人 '{raw_bot_name}'"))
}
