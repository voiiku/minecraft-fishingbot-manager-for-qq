//! `/log`：读取某个机器人最近一次运行的日志（已过滤垃圾游戏信息的那一份）。

use crate::config;
use crate::utils::{BotDirEntry, Report, latest_log_path, read_last_lines};

/// 读取机器人最新日志的最后 N 行（`/log`，仅私聊）。
///
/// 权限在本函数内由 [`BotDirEntry::find_for_caller`] 按目录归属判定（管理员可跨目录），路由层不查。
/// 读取启动流程创建日志文件时写入 `<机器人目录>/<runtime.logs_dir>/时间戳.log` 的日志
/// （见 [`crate::utils::bot_log_files::create_bot_log_file`]）：该文件在写盘前已经过
/// `grep -v -E`（配置 `keywords.filter_regex`，匹配的行会被丢弃），因此 `+24.50XP` 这类高频日志行不会出现。
/// FishingBot 自己那份日志**不参与挑选**（它在同层 `raw/` 里，且是指向 `/dev/null` 的链接），原因见
/// [`crate::utils::bot_log_files`] 的模块文档。
///
/// 日志较长时由 [`Report::render`] 按 `runtime.max_reply_chars` 截断，并注明省略行数。
pub fn read_bot_log(
    caller_qq: i64,
    raw_bot_name: &str,
    line_count: usize,
) -> Result<String, String> {
    // 1. 定位机器人目录：存在性与权限一起判定（口径＝目录归属，管理员可跨目录）。
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    if line_count == 0 {
        return Err("读取行数必须大于 0。".to_string());
    }
    // 2. 实际行数受上限约束：一条指令不能把整份大日志拉回来。
    let target_line_count = line_count.min(config::get().runtime.max_log_lines);

    // 3. 只读最新那一份日志文件。
    let Some(target_log_path) = latest_log_path(&bot_dir_entry.bot_dir_path) else {
        return Err(format!(
            "机器人 '{raw_bot_name}' 暂无日志；机器人启动后才会生成日志。"
        ));
    };

    // 4. 只保留最后 N 行（滑动窗口，见 read_last_lines）。
    let log_lines = read_last_lines(&target_log_path, target_line_count)?;

    let file_name = target_log_path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("日志");

    let mut report = Report::new(format!("机器人 '{raw_bot_name}' 日志 [{file_name}]"));

    if log_lines.is_empty() {
        report.basics.push("日志文件为空。".to_string());
        return Ok(report.render());
    }

    // 日志原文走整块内容，避免每行都被加上列表前缀。
    report.basics.push(format!("共 {} 行", log_lines.len()));
    report.content = Some(log_lines.join("\n"));

    Ok(report.render())
}
