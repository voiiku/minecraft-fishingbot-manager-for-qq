//! `/startall`：批量启动所有未运行的机器人（仅管理员）。

use crate::utils::{
    LaunchTiming, all_bot_dir_entries, split_bot_entries_by_running,
    split_bot_entries_by_token_status, start_bot_sessions,
};

/// 启动全部未运行的机器人（`/startall`）。
///
/// 本函数由路由 [`crate::handle_all_bots_control`] 调用，权限在路由层校验
/// 名单来自 [`all_bot_dir_entries`] + 两个分组原语：先按 Token 就绪拆开，再把可启动的按运行状态拆开。
/// 拉起与复核交给 [`start_bot_sessions`]。
///
/// 缺 Token 的账号**不是失败**（它们是"还没配好"），因此只进跳过明细，让使用者看得见；
/// 真要拉起时缺 Token 的目标也会被 [`start_bot_sessions`] 拒绝，不会白起一个 JVM。
pub fn start_all_bots() -> Result<String, String> {
    let (token_ready_bot_entries, missing_token_bot_entries) =
        split_bot_entries_by_token_status(&all_bot_dir_entries()?);
    let token_ready_bot_count = token_ready_bot_entries.len();
    let (running_bot_entries, not_running_bot_entries) =
        split_bot_entries_by_running(&token_ready_bot_entries);

    // 没有可启动对象时直接给结论，不必生成空报告；缺 Token 的账号顺口提示一句。
    if token_ready_bot_count == 0 {
        return Ok(if missing_token_bot_entries.is_empty() {
            "全部启动执行结果: 未配置任何有效账号，未启动。".to_string()
        } else {
            format!(
                "全部启动执行结果: 未配置任何有效账号，未启动。\n（另有 {} 个账号缺少 Token 文件，先用 /token <机器人名> <token> 配置）",
                missing_token_bot_entries.len()
            )
        });
    }
    if not_running_bot_entries.is_empty() {
        return Ok("全部启动执行结果: 全部有效机器人均已运行，无需启动。".to_string());
    }

    // 批量拉起 + 复核，与 /start 共用同一份实现。
    let mut report = start_bot_sessions(&not_running_bot_entries, LaunchTiming::batch());
    report.set_title("全部启动执行结果");

    // 计数行的顺序与回复里出现的顺序一致（见 Report::set_counts）。
    let sent_start_command_count = report.succeeded.len();
    let start_failure_count = report.failed.len();
    report.set_counts(&[
        ("有效账号总数", token_ready_bot_count),
        ("已发送启动命令", sent_start_command_count),
        ("跳过 (已在运行)", running_bot_entries.len()),
        ("跳过 (缺少 Token)", missing_token_bot_entries.len()),
        ("启动失败", start_failure_count),
    ]);

    // 没被启动的账号都要能看见：已在运行的不算失败，缺 Token 的也不是失败。
    report.skipped = running_bot_entries
        .iter()
        .map(|bot_dir_entry| format!("机器人 {}: 已在运行，未重复启动", bot_dir_entry.bot_name))
        .chain(missing_token_bot_entries.iter().map(|bot_dir_entry| {
            format!(
                "机器人 {}: 缺少 Token 文件，未启动（先 /token {} <token>）",
                bot_dir_entry.bot_name, bot_dir_entry.bot_name
            )
        }))
        .collect();

    Ok(report.render())
}
