//! `/restartall`：批量重启所有运行中的机器人（仅管理员）。

use crate::utils::{
    StartupQueue, all_bot_dir_entries, restart_bot_sessions, split_bot_entries_by_running,
};

/// 重启全部运行中的机器人（`/restartall`）。
///
/// 本函数由路由 [`crate::handle_all_bots_control`] 调用，权限在路由层校验
/// 与 `/restart` 共用 [`restart_bot_sessions`]：先停止，再错峰启动，最后统一复核。
pub fn restart_all_bots() -> Result<String, String> {
    // 只看「在运行」的那批：未运行的目标重启等于启动，那是 /startall 的事。
    // Token 过滤不在本函数：restart_bot_sessions 会在停止前排除缺 Token 的目标并写入失败明细。
    // 重启包含停止阶段；先停止无法重新启动的账号会使其掉线。
    let (running_bot_entries, _) = split_bot_entries_by_running(&all_bot_dir_entries()?);
    let (starting_entries, running_bot_entries): (Vec<_>, Vec<_>) = running_bot_entries
        .into_iter()
        .partition(|entry| StartupQueue::bot_in_queue(&entry.bot_name));
    let running_bot_count = running_bot_entries.len();

    if running_bot_count == 0 {
        return Ok(format!(
            "全部重启结果：没有可重启的运行中机器人，无需重启。\n跳过正在启动：{} 台",
            starting_entries.len()
        ));
    }

    let mut report = restart_bot_sessions(&running_bot_entries);
    report.set_title("全部重启结果");
    report.skipped.extend(
        starting_entries
            .iter()
            .map(|entry| format!("机器人 {}：正在启动，未重复重启。", entry.bot_name)),
    );

    // 计数行的顺序与回复里出现的顺序一致（见 Report::set_counts）。
    // 与 /startall 同一口径：只统计日志确认进服的成功结果。
    let sent_restart_command_count = report.succeeded.len();
    let restart_failure_count = report.failed.len();
    report.set_counts(&[
        ("运行中账号", running_bot_count),
        ("已重启并确认进服", sent_restart_command_count),
        ("跳过（正在启动）", starting_entries.len()),
        ("重启失败", restart_failure_count),
    ]);

    Ok(report.render())
}
