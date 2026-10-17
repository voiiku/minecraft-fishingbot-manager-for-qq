//! `/show allbots`：列出全服机器人及 Token 配置情况。

use crate::config;
use crate::utils::{Report, all_bot_dir_entries};

/// 列出全服机器人，逐条标注 Token 配置状态。所有用户均可查询。
pub fn all_bots_list() -> Result<String, String> {
    let mut report = Report::new("全服机器人列表");

    // 根目录不存在时给结论而不是报错：这是配置问题，按提示改配置即可。
    if !config::get().paths.bots_dir.is_dir() {
        report
            .basics
            .push("机器人根目录不存在，请检查配置里的 paths.base_dir".to_string());
        return Ok(report.render());
    }

    // 机器人目录口径与 /show mybots、/startall 完全一致（见 utils::all_bot_dir_entries）。
    let bot_dir_entries = all_bot_dir_entries()?;

    let mut rows: Vec<String> = Vec::new();
    let mut ready_token_count = 0usize;
    let mut missing_token_count = 0usize;

    for bot_dir_entry in bot_dir_entries {
        if bot_dir_entry.bot_token_status.is_ready() {
            ready_token_count += 1;
        } else {
            missing_token_count += 1;
        }

        rows.push(format!(
            "机器人: {} (所属QQ: {}, Token: {})",
            bot_dir_entry.bot_name, bot_dir_entry.owner_qq, bot_dir_entry.bot_token_status
        ));
    }

    if rows.is_empty() {
        report.basics.push("系统中暂无任何机器人配置".to_string());
        return Ok(report.render());
    }

    report
        .basics
        .push(format!("已配置 Token: {ready_token_count} 个"));
    report
        .basics
        .push(format!("待添加 Token: {missing_token_count} 个"));
    report.basics.extend(rows);

    Ok(report.render())
}
