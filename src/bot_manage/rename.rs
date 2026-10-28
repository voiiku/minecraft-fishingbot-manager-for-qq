//! `/bot rename`：重命名机器人目录。

use std::fs;

use crate::utils::{
    BotDirEntry, StartupQueue, ToCleanBotName, forget_bot_pause_status, is_bot_name_taken,
    refusal_if_running,
};

/// 重命名机器人（`/bot rename`）。
///
/// 权限在本函数内由 [`BotDirEntry::find_for_caller`] 按目录归属判定（创建者或管理员）。
/// 新旧名字都必须是合法机器人名（[`ToCleanBotName::to_clean_bot_name`]）：非法就直接报错，不再静默过滤。
pub fn rename_bot(
    caller_qq: i64,
    raw_old_bot_name: &str,
    raw_new_bot_name: &str,
) -> Result<String, String> {
    // 目录名一律用清洗后的名字，因此这里先算好，后面的比较与拼接都基于它。
    let clean_old_bot_name = raw_old_bot_name.to_clean_bot_name()?;
    let clean_new_bot_name = raw_new_bot_name.to_clean_bot_name()?;

    if clean_old_bot_name == clean_new_bot_name {
        return Err("新名称与原名称相同，无需修改。".to_string());
    }

    // 定位旧目录（口径＝目录归属）：跨目录改名仅管理员可执行。
    let old_bot_dir = BotDirEntry::find_for_caller(raw_old_bot_name, caller_qq)?.bot_dir_path;

    if StartupQueue::bot_in_queue(&clean_old_bot_name) {
        return Err(format!(
            "机器人 '{clean_old_bot_name}' 已在启动队列中；请先执行 /stop {clean_old_bot_name}，再重命名。"
        ));
    }

    if let Some(refusal) = refusal_if_running(&clean_old_bot_name, "重命名") {
        return Err(refusal);
    }

    // 确认账号已停止后再检查全服重名。
    if is_bot_name_taken(&clean_new_bot_name) {
        return Err(format!(
            "机器人名 '{clean_new_bot_name}' 已存在；可使用 /show allbots 查看全部机器人。"
        ));
    }

    // 新目录与旧目录同属一个 QQ 账号，因此只在父目录里换名字。
    let owner_dir = old_bot_dir
        .parent()
        .ok_or_else(|| "无法获取原机器人目录的父级路径。".to_string())?;
    let new_bot_dir = owner_dir.join(&clean_new_bot_name);

    fs::rename(&old_bot_dir, &new_bot_dir).map_err(|e| format!("重命名失败：{e}"))?;
    forget_bot_pause_status(&clean_old_bot_name);
    forget_bot_pause_status(&clean_new_bot_name);

    Ok(format!(
        "已将机器人 '{clean_old_bot_name}' 重命名为 '{clean_new_bot_name}'。"
    ))
}
