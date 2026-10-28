//! `/command`：向单个运行中的机器人发送游戏内指令。

use crate::utils::{BotDirEntry, is_bot_screen_running, send_command_to_single_bot};

/// 把插件入口切好的**原始片段**拼成一条游戏内指令（`/command` 与 `/commandall` 共用）。
///
/// 片段用单空格连接；一个片段都没有（拼完为空）就 `Err`，文案可直接回复。
/// 拼接口径只写这一处 —— 插件入口只负责切分，不再替业务决定参数长什么样。
pub fn join_bot_command(raw_bot_command_args: &[&str]) -> Result<String, String> {
    let bot_command = raw_bot_command_args.join(" ");

    if bot_command.trim().is_empty() {
        return Err("指令内容不能为空。".to_string());
    }
    if bot_command.chars().any(char::is_control) {
        return Err("指令内容不能包含换行符或控制字符。".to_string());
    }

    Ok(bot_command)
}

/// 向运行中的机器人发送游戏内指令（`/command`）。
///
/// 例如发送 `/home fishing` 让机器人返回钓鱼点。
/// `raw_bot_command_args` 是**插件入口原样透传的片段**，先由 [`join_bot_command`] 拼成一条指令；
/// 校验在这里做（权限在业务层，不在路由层）：目录归属由 [`BotDirEntry::find_for_caller`] 判定，
/// 另外要求会话在跑。真正的注入是 screen 原语 [`send_command_to_single_bot`]。
pub fn send_command_to_bot(
    raw_bot_name: &str,
    caller_qq: i64,
    raw_bot_command_args: &[&str],
) -> Result<String, String> {
    let bot_command = join_bot_command(raw_bot_command_args)?;

    // 只要"目录存在 + 调用者有权限"这个结论，返回的条目用不上（Token 检查在 /start 里做）。
    let bot = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;
    let bot_name = &bot.bot_name;

    // 会话不存在就没有可注入的对象，先拦下，避免指令被塞进不存在的会话。
    if !is_bot_screen_running(bot_name) {
        return Err(format!("机器人 '{bot_name}' 未运行，无法发送指令。"));
    }

    send_command_to_single_bot(bot_name, &bot_command)?;

    Ok(format!("已向机器人 '{bot_name}' 发送指令：{bot_command}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 多词指令用单空格连接；一个片段都没有时按"空指令"拒绝。
    #[test]
    fn bot_command_is_joined_by_single_space() {
        assert_eq!(
            join_bot_command(&["/home", "fishing"]),
            Ok("/home fishing".to_string())
        );
        assert!(join_bot_command(&[]).is_err());
        assert!(join_bot_command(&[" ", ""]).is_err());
        for command in ["/pause\n/stop", "/pause\r", "\u{3}", "\u{1b}"] {
            assert!(join_bot_command(&[command]).is_err());
        }
    }
}
