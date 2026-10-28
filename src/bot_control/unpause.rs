//! `/unpause`：取消暂停、恢复钓鱼（与 `/pause` 配对）。

use crate::bot_control::command::send_command_to_bot;
use crate::utils::{BotDirEntry, confirm_pause_command};

/// 取消暂停、恢复钓鱼（`/unpause`）。
///
/// 与 `/pause` 对称：同样只是给机器人发一条游戏内指令（常量本身就是完整的一条）。
pub fn unpause_fishing(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    let bot = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;
    let mut sent = String::new();
    let confirmed = confirm_pause_command(&bot, false, || {
        sent = send_command_to_bot(&bot.bot_name, caller_qq, &["/unpause"])?;
        Ok(())
    })?;
    Ok(if confirmed {
        format!("机器人 '{}' 已恢复钓鱼。", bot.bot_name)
    } else {
        sent
    })
}
