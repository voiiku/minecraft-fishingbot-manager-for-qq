//! `/commandall`：把同一条游戏内指令发给所有运行中的机器人（仅管理员）。

use crate::bot_control::command::join_bot_command;
use crate::utils::{
    Report, running_bot_names, send_command_to_single_bot, token_ready_bot_dir_entries,
};

/// 向所有运行中的机器人发送同一条指令（`/commandall`）。
///
/// 本函数由路由 [`crate::handle_all_bots_control`] 调用，权限在路由层校验。
/// `raw_bot_command_args` 是插件入口原样透传的片段，拼接与校验走 [`join_bot_command`]
/// （与 `/command` 同一口径，只写一处）。
pub fn send_command_to_all_bots(raw_bot_command_args: &[&str]) -> Result<String, String> {
    let bot_command = join_bot_command(raw_bot_command_args)?;

    // 名单口径与 /status 一致：只发给「配好 Token 且正在运行」的机器人。
    let running_bot_names = running_bot_names(&token_ready_bot_dir_entries()?);
    let running_bot_count = running_bot_names.len();

    let mut succeeded_bot_count = 0usize;
    let mut failed_messages: Vec<String> = Vec::new();
    for bot_name in running_bot_names {
        match send_command_to_single_bot(&bot_name, &bot_command) {
            Ok(_) => succeeded_bot_count += 1,
            Err(message) => failed_messages.push(message),
        }
    }

    let mut report = Report::new("全部发送指令结果");
    report.set_counts(&[
        ("运行中账号", running_bot_count),
        ("成功发送", succeeded_bot_count),
        ("发送失败", failed_messages.len()),
    ]);
    // 「发的是什么」插在计数行中间：先看名单规模，再看发的内容，最后看结果。
    report.basics.insert(1, format!("发送指令: {bot_command}"));
    report.failed = failed_messages;

    Ok(report.render())
}
