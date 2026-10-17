//! `/status`：汇总当前运行中机器人的状态。
//!
//! 表头各列的顺序与数据行的字段顺序是一份隐式契约（都用 ` | ` 连接），改一处必须同步另一处。

use crate::utils::{Report, format_uptime, running_bots_status};

/// 生成状态报告。
pub fn status() -> Result<String, String> {
    let running_bot_infos = running_bots_status()?;

    let mut report = Report::new("机器人运行状态");

    // 一个都没在跑时直接给结论，不输出空表。
    if running_bot_infos.is_empty() {
        report.basics.push("目前没有正在运行的机器人".to_string());
        return Ok(report.render());
    }

    report
        .basics
        .push(format!("共 {} 个", running_bot_infos.len()));
    // 列顺序：机器人名 | 服务器状态 | 暂停状态 | 运行时长（与下面的数据行一一对应）。
    report
        .basics
        .push("机器人 | 服务器状态 | 暂停状态 | 运行时长".to_string());

    let rows: Vec<String> = running_bot_infos
        .into_iter()
        .map(|bot_info| {
            format!(
                "{} | {} | {} | {}",
                bot_info.bot_name,
                bot_info.connection_status,
                bot_info.pause_status,
                format_uptime(bot_info.running_seconds)
            )
        })
        .collect();
    report.basics.extend(rows);

    Ok(report.render())
}
