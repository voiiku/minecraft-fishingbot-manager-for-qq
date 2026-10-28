//! `/start`：启动单个机器人。

use crate::utils::{BotDirEntry, StartupQueue, is_bot_screen_running, start_bot_sessions};

/// 启动单个机器人（`/start`）。
///
/// 校验都在本函数内做（权限在业务层，不在路由层）：定位目录、确认 Token 就绪、确认未在运行；
/// 其中"目录存在 + 调用者有权限"由 [`BotDirEntry::find_for_caller`] 一次判定（口径＝目录归属，
/// 管理员可跨目录）。
/// 启动与复核交给与 `/startall` 共用的 [`start_bot_sessions`]。
///
/// 只有本次运行日志确认进服且会话仍存活，才报告启动成功。
pub fn start_bot(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    // 目录存在 + 调用者有权限，两者都由这一句保证。
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    // 已经在队列里等待的，不重复排队（`/stop` 可以把排队中的撤下来）。
    if StartupQueue::bot_in_queue(&bot_dir_entry.bot_name) {
        return Err(format!(
            "机器人 '{raw_bot_name}' 已在启动队列中；请等待启动，或执行 /stop {raw_bot_name} 取消排队。"
        ));
    }
    if is_bot_screen_running(raw_bot_name) {
        return Err(format!("机器人 '{raw_bot_name}' 已在运行，无需启动。"));
    }
    if !bot_dir_entry.bot_token_status.is_ready() {
        return Err(format!(
            "机器人 '{raw_bot_name}' 缺少 Token，无法启动；请先执行 /bot login {raw_bot_name}。"
        ));
    }

    // 与 `/startall` 共用启动队列：排队、批间等待、启动与复核都在队列里完成。
    let report = start_bot_sessions(&[bot_dir_entry]);
    if !report.is_ok() {
        return Err(report.failure_text());
    }
    if report.succeeded.is_empty() {
        // 并发场景下可能已被其他指令启动或加入队列：将跳过原因返回给用户。
        return Err(report.skipped.join(" / "));
    }

    Ok(format!("已启动机器人 '{raw_bot_name}'，日志已确认进服。"))
}
