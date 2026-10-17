//! `/unpause`：取消暂停、恢复钓鱼（与 `/pause` 配对）。

use crate::bot_control::command::send_command_to_bot;

/// 取消暂停、恢复钓鱼（`/unpause`）。
///
/// 与 `/pause` 对称：同样只是给机器人发一条游戏内指令（常量本身就是完整的一条）。
pub fn unpause_fishing(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    send_command_to_bot(raw_bot_name, caller_qq, &["/unpause"])
}
