//! `/status`：汇总当前运行中机器人的状态。
//!
//! 表头各列的顺序与数据行的字段顺序是一份隐式契约（都用 ` | ` 连接），改一处必须同步另一处。

use crate::utils::{Report, StartupQueue, format_uptime, running_bots_status};

/// 生成状态报告。
pub fn status() -> Result<String, String> {
    let running_bot_infos = running_bots_status()?;

    let mut report = Report::new("运行状态");

    // 启动队列中的机器人尚未运行但已排队：单独列出，避免被误认为丢失。
    let waiting_names = StartupQueue::waiting_bot_names();
    let active_names = StartupQueue::active_bot_names();
    report.queued = waiting_names
        .iter()
        .map(|bot_name| format!("{bot_name}：等待启动"))
        .chain(
            active_names
                .iter()
                .map(|bot_name| format!("{bot_name}：正在启动或复核")),
        )
        .collect();

    // 没有任何机器人在运行时直接给出结论，不输出空表。
    if running_bot_infos.is_empty() {
        report.basics.push("当前没有运行中的机器人。".to_string());
        return Ok(report.render());
    }

    report
        .basics
        .push(format!("共 {} 台", running_bot_infos.len()));
    // 列顺序：机器人名 | 服务器状态 | 暂停状态 | 运行时长（与后续数据行一一对应）。
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
