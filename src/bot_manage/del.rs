//! `/bot del`：删除机器人及其数据目录。

use std::fs;

use crate::utils::{BotDirEntry, StartupQueue, refusal_if_running};

/// 删除机器人及其数据目录（`/bot del`）。
///
/// 权限（目录归属）与"这个名字在不在"都由 [`BotDirEntry::find_for_caller`] 一次判定：
/// 名字非法时回非法原因，找不到或无权时回统一的"未找到"文案（刻意不区分、也不暴露归属）。
/// 名字只在那里清洗一次，回复与后台日志都用条目里的干净名字。
pub fn delete_bot(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    // 运行中的机器人必须先停止，否则会删除它正在写入的数据目录。
    if let Some(refusal) = refusal_if_running(&bot_dir_entry.bot_name, "删除") {
        return Err(refusal);
    }

    // 排队中或正在启动的也不能删：启动线程随后会往一个不存在的目录里启动 JVM。
    if StartupQueue::bot_in_queue(&bot_dir_entry.bot_name) {
        return Err(format!(
            "机器人 '{}' 已在启动队列中；请先执行 /stop {} 取消排队，再删除。",
            bot_dir_entry.bot_name, bot_dir_entry.bot_name
        ));
    }

    fs::remove_dir_all(&bot_dir_entry.bot_dir_path).map_err(|e| {
        log::warn!("机器人 {} 删除失败: {e}", bot_dir_entry.bot_name);
        format!("删除机器人 '{}' 失败：\n{e}", bot_dir_entry.bot_name)
    })?;

    log::info!("机器人 {} 删除成功", bot_dir_entry.bot_name);
    Ok(format!(
        "已删除机器人 '{}' 及其全部数据。",
        bot_dir_entry.bot_name
    ))
}
