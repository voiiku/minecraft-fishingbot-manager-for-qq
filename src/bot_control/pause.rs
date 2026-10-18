//! `/pause`：暂停钓鱼但保持在线。

use crate::bot_control::command::send_command_to_bot;

/// 暂停钓鱼但保持在线（`/pause`）。
///
/// 本质是给机器人发一条游戏内指令，因此校验与注入都复用 [`send_command_to_bot`]。
/// 常量本身即为完整指令，可直接作为"原始片段"传入，无需使用用户文本的拼接逻辑。
pub fn pause_fishing(raw_bot_name: &str, caller_qq: i64) -> Result<String, String> {
    send_command_to_bot(raw_bot_name, caller_qq, &["/pause"])
}
