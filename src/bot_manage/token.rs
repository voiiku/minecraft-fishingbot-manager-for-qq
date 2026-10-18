//! `/token`：写入机器人的 refreshToken 文件（仅私聊）。

use std::fs;

use crate::config;
use crate::utils::{BotDirEntry, StartupQueue, refusal_if_running};

/// 写入机器人的 refreshToken 文件（`/token`，仅私聊）。不校验 Token 有效性。
///
/// 权限在本函数内由 [`BotDirEntry::find_for_caller`] 按目录归属判定（管理员可跨目录）。
/// `raw_token` 是**插件入口原样透传的值**（切分时已去除首尾空白），写入前不再修改。
///
/// 运行中的账号**禁止改 Token**：正在跑的 JVM 只在启动时读一次 Token 文件，改文件对它没有任何
/// 影响，却容易让人以为"已经换好了"。要换就先 `/stop`（与 `/bot del`、`/bot rename` 同一条规矩）。
pub fn update_bot_token_file(
    raw_bot_name: &str,
    raw_token: String,
    caller_qq: i64,
) -> Result<String, String> {
    // 定位目录：存在性与权限一起判定（口径＝目录归属），Token 只会落到调用者有权访问的目录里。
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    // 排队中的先撤单：队列里的条目带着旧的 Token 状态，写完后应当重新 /start。
    if StartupQueue::bot_in_queue(&bot_dir_entry.bot_name)
        && !StartupQueue::remove_waiting_bot(&bot_dir_entry.bot_name)
    {
        return Err(format!("机器人 '{raw_bot_name}' 正在启动，请稍后重试。"));
    }

    if let Some(refusal) = refusal_if_running(raw_bot_name, "更新 Token") {
        return Err(refusal);
    }

    let token_file_path = bot_dir_entry
        .bot_dir_path
        .join(&config::get().runtime.token_file);

    // `fs::write` 在文件不存在时创建，存在时覆盖；内容原样写入。
    fs::write(&token_file_path, &raw_token)
        .map_err(|e| format!("更新机器人 '{raw_bot_name}' 的 Token 文件失败：{e}"))?;

    let message = format!("已更新机器人 '{raw_bot_name}' 的 Token 文件。");
    log::info!("{message}");
    Ok(message)
}
