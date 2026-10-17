//! `/restartall`：批量重启所有运行中的机器人（仅管理员）。

use crate::utils::{
    LaunchTiming, all_bot_dir_entries, restart_bot_sessions, split_bot_entries_by_running,
};

/// 重启全部运行中的机器人（`/restartall`）。
///
/// 本函数由路由 [`crate::handle_all_bots_control`] 调用，权限在路由层校验
/// 与 `/restart` 共用 [`restart_bot_sessions`]：先停止，再错峰拉起，最后统一复核。
pub fn restart_all_bots() -> Result<String, String> {
    // 只看「在运行」的那批：未运行的目标重启等于启动，那是 /startall 的事。
    // Token 口径不在这里 —— 缺 Token 的目标由 restart_bot_sessions 在**停止之前**剔掉并写进失败明细
    //（重启是先停再起，白停一次会把一个可能还在钓鱼的账号弄下线）。
    let (running_bot_entries, _) = split_bot_entries_by_running(&all_bot_dir_entries()?);
    let running_bot_count = running_bot_entries.len();

    if running_bot_count == 0 {
        return Ok("全部重启执行结果: 没有运行的机器人实例，无需重启。".to_string());
    }

    let mut report = restart_bot_sessions(&running_bot_entries, LaunchTiming::batch());
    report.set_title("全部重启执行结果");

    // 计数行的顺序与回复里出现的顺序一致（见 Report::set_counts）。
    // 计数标签与 /startall 同一口径：拉起部分只能说"已发送重启命令"。
    let sent_restart_command_count = report.succeeded.len();
    let restart_failure_count = report.failed.len();
    report.set_counts(&[
        ("运行中账号", running_bot_count),
        ("已发送重启命令", sent_restart_command_count),
        ("重启失败", restart_failure_count),
    ]);

    Ok(report.render())
}
