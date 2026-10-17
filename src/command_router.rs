//! kovi 插件入口：解析 QQ 消息里的指令、把参数原样切分好，再交由 `mc_qq_fishbot` 库执行。
//!
//! 只负责"认指令 → 洗参数 → 分发"：群聊走 `/xxx` 指令，私聊只放行 `/help`、`/token` 与 `/log`；
//! **不查权限** —— 批量指令的权限在库的路由层（`handle_all_bots_control`）判一次，
//! 单机器人指令的权限在业务层按目录归属判定（见 `src/lib.rs` 的模块文档）。
//! 业务动作都在库里，便于单独阅读与测试。
//!
//! **这里只切分，不清洗**：消息按空白切成片段后原样往下传，谁真正要用就由谁清洗一次
//! （机器人名见 `ToCleanBotName::to_clean_bot_name`，游戏内指令片段见
//! `bot_control::command::join_bot_command`）；参数名以 `raw_` / `clean_` 标明形态，
//! 避免"到底收到的是原始还是清洗过的"这种疑问。
//! 数字例外：`/log` 的行数、`/ping` 的端口在本文件就解析成数字 —— 解析失败要当场回用法提示。
//!
//! 心跳事件（[`HeartbeatEvent`]）在 [`crate::heartbeat_event`] 里单独实现，本文件只负责注册它
//! 并拉起库里的心跳看门狗。
//!
//! 插件入口在一个进程里只会被框架调用一次（本程序不重建 Bot：连接一断就收工退出，
//! 交给 systemd 重新拉起），因此这里不需要幂等守卫。

use kovi::PluginBuilder;
use kovi_onebot::EventRegistrar;
use mc_qq_fishbot::{
    AllBotsControlAction, BotControlAction, BotManageAction, ShowAction, handle_all_bots_control,
    handle_bot_control, handle_bot_log, handle_bot_manage, handle_checkserver, handle_help,
    handle_ping, handle_show, handle_status, note_heartbeat, watch_heartbeat,
};

use crate::heartbeat_event::HeartbeatEvent;

#[kovi::plugin]
pub async fn main() {
    PluginBuilder::on_group_msg(|event| async move {
        let Some(text) = event.borrow_text() else {
            return;
        };
        let Some((command, args)) = parse_command(text) else {
            return;
        };

        if let Some(reply) = handle_group_command(command, &args, event.user_id).await {
            event.reply(reply);
        }
    });

    PluginBuilder::on_private_msg(|event| async move {
        let Some(text) = event.borrow_text() else {
            return;
        };
        let Some((command, args)) = parse_command(text) else {
            return;
        };

        event.reply(handle_private_command(command, &args, event.user_id));
    });

    // 心跳：交给库里的看门狗（只有有效心跳才刷新最后心跳时刻，规则见 `heartbeat`）。
    // 用 kovi::spawn 而不是 tokio::spawn：长驻任务交给框架登记，插件停用时能一起收掉。
    PluginBuilder::on::<HeartbeatEvent, _>(|event| async move {
        note_heartbeat(event.report);
    });
    kovi::spawn(watch_heartbeat());
}

/// 把消息文本切成「指令 + 参数」：只认 `/xxx` 指令，其余消息返回 `None`。
///
/// 群聊与私聊入口共用，两条入口的指令识别口径（去空白、按空白切分）不会跑偏。
fn parse_command(text: &str) -> Option<(&str, Vec<&str>)> {
    let mut parts = text.split_whitespace();
    let command = parts.next()?;

    command.starts_with('/').then(|| (command, parts.collect()))
}

/// 构造用法错误提示。
fn usage(example: &str) -> String {
    format!("用法错误：{example}")
}

/// 分发群聊指令；未识别的指令返回 `None`，由调用方忽略。
async fn handle_group_command(command: &str, args: &[&str], caller_qq: i64) -> Option<String> {
    let reply = match command {
        "/help" => {
            if args.is_empty() {
                handle_help()
            } else {
                usage("/help")
            }
        }
        "/status" => {
            if args.is_empty() {
                handle_status()
            } else {
                usage("/status")
            }
        }
        "/checkserver" => {
            if args.is_empty() {
                handle_checkserver().await
            } else {
                usage("/checkserver")
            }
        }
        "/ping" => {
            if args.len() > 2 {
                usage("/ping [地址] [端口]")
            } else {
                let host = args.first().copied();
                let port = args.get(1).and_then(|p| p.parse().ok());
                handle_ping(host, port).await
            }
        }

        // 单个机器人的控制指令。
        "/start" => bot_control_reply(BotControlAction::Start, command, args, caller_qq),
        "/stop" => bot_control_reply(BotControlAction::Stop, command, args, caller_qq),
        "/restart" => bot_control_reply(BotControlAction::Restart, command, args, caller_qq),
        "/pause" => bot_control_reply(BotControlAction::Pause, command, args, caller_qq),
        "/unpause" => bot_control_reply(BotControlAction::Unpause, command, args, caller_qq),
        // 向机器人发送游戏内指令：只切分，不拼接、不清洗 —— 原始片段在业务函数里拼一次。
        "/command" => {
            if args.len() < 2 {
                usage("/command <机器人名> <指令内容>")
            } else {
                handle_bot_control(BotControlAction::Command(&args[1..]), args[0], caller_qq)
            }
        }
        // 机器人增删与改名。
        "/bot" => handle_bot_manage_args(args, caller_qq),
        // 信息查询。
        "/show" => {
            if args.len() != 1 {
                usage("/show <config|mybots|allbots|admins>")
            } else {
                match ShowAction::from_name(args[0]) {
                    Some(action) => handle_show(action, caller_qq),
                    None => usage("/show <config|mybots|allbots|admins>"),
                }
            }
        }
        // 全服批量指令（仅管理员）：权限由库的路由层 handle_all_bots_control 校验，这里只切参数。
        "/startall" => {
            all_bots_control_reply(AllBotsControlAction::StartAll, command, args, caller_qq)
        }
        "/stopall" => {
            all_bots_control_reply(AllBotsControlAction::StopAll, command, args, caller_qq)
        }
        "/restartall" => {
            all_bots_control_reply(AllBotsControlAction::RestartAll, command, args, caller_qq)
        }
        "/commandall" => {
            if args.is_empty() {
                usage("/commandall <要发送的指令字符串>")
            } else {
                handle_all_bots_control(AllBotsControlAction::CommandAll(args), caller_qq)
            }
        }
        _ => return None,
    };

    Some(reply)
}

/// 私聊里应该回的那句提示：私聊只放行下面这三条指令。
const PRIVATE_ONLY_HINT: &str = "该指令只能在群聊中使用；私聊可用：/help、/token、/log。";

/// 完全不认识的指令统一回这一句（不区分群里还是私聊的用户输入）。
const UNKNOWN_COMMAND_REPLY: &str = "错误的指令，输入/help 开始使用";

/// 私聊指令分发：只放行 `/help`、`/token`、`/log`，其余按"能识别但不给用"与"不认识"分别回话。
fn handle_private_command(command: &str, args: &[&str], caller_qq: i64) -> String {
    match command {
        // 帮助菜单：群里与私聊都能用。
        "/help" => {
            if args.is_empty() {
                handle_help()
            } else {
                usage("/help")
            }
        }
        "/token" => {
            if args.len() != 2 {
                usage("/token <机器人名> <token字符串>")
            } else {
                // Token 是单个参数，原样透传（切分时首尾空白已经掉了），写入前不再改写。
                handle_bot_manage(
                    BotManageAction::Token(args[1].to_string()),
                    args[0],
                    caller_qq,
                )
            }
        }
        "/log" => {
            if args.len() != 2 {
                usage("/log <机器人名> <日志行数>")
            } else {
                match args[1].parse::<usize>() {
                    Ok(lines) => handle_bot_log(caller_qq, args[0], lines),
                    Err(_) => usage("/log <机器人名> <日志行数>"),
                }
            }
        }
        // 能识别、但只在群聊提供：Token 与日志属于敏感内容，其余指令在群里发。
        "/status" | "/checkserver" | "/ping" | "/start" | "/stop" | "/restart" | "/pause"
        | "/unpause" | "/command" | "/bot" | "/show" | "/startall" | "/stopall" | "/restartall"
        | "/commandall" => PRIVATE_ONLY_HINT.to_string(),
        // 不认识：只说一句怎么继续，不猜用户想干什么。
        _ => UNKNOWN_COMMAND_REPLY.to_string(),
    }
}

/// 解析 `/bot` 子命令：add / del / rename。
fn handle_bot_manage_args(args: &[&str], caller_qq: i64) -> String {
    let Some(action) = args.first() else {
        return usage("/bot <add|del|rename> <机器人名>");
    };

    match *action {
        "add" => bot_manage_reply(BotManageAction::Add, args, caller_qq),
        "del" => bot_manage_reply(BotManageAction::Delete, args, caller_qq),
        "rename" => {
            if args.len() != 3 {
                usage("/bot rename <旧机器人名> <新机器人名>")
            } else {
                let new_name = args[2].to_string();
                handle_bot_manage(BotManageAction::Rename(new_name), args[1], caller_qq)
            }
        }
        _ => usage("/bot <add|del|rename> <机器人名>"),
    }
}

/// 单机器人控制指令的公共部分：这类指令只带一个机器人名（权限在业务层按目录归属判定，本层不查）。
fn bot_control_reply(
    action: BotControlAction,
    command: &str,
    args: &[&str],
    caller_qq: i64,
) -> String {
    if args.len() != 1 {
        usage(&format!("{command} <机器人名>"))
    } else {
        handle_bot_control(action, args[0], caller_qq)
    }
}

/// add / del 子命令的公共部分：这两个只带一个机器人名（权限在业务层按目录归属判定，本层不查）。
fn bot_manage_reply(action: BotManageAction, args: &[&str], caller_qq: i64) -> String {
    if args.len() != 2 {
        usage("/bot add|del <机器人名>")
    } else {
        handle_bot_manage(action, args[1], caller_qq)
    }
}

/// 全服批量指令的公共部分：这些指令不接受参数。权限由 [`handle_all_bots_control`] 校验（本层不看权限）。
fn all_bots_control_reply(
    action: AllBotsControlAction,
    command: &str,
    args: &[&str],
    caller_qq: i64,
) -> String {
    if args.is_empty() {
        handle_all_bots_control(action, caller_qq)
    } else {
        usage(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 私聊里"只能在群聊用"的指令与"完全不认识"的指令要分别回对应的话。
    #[test]
    fn private_commands_get_the_right_reply() {
        let group_only_commands = [
            "/status",
            "/checkserver",
            "/ping",
            "/start",
            "/stop",
            "/restart",
            "/pause",
            "/unpause",
            "/command",
            "/bot",
            "/show",
            "/startall",
            "/stopall",
            "/restartall",
            "/commandall",
        ];
        for command in group_only_commands {
            assert_eq!(
                handle_private_command(command, &[], 1),
                PRIVATE_ONLY_HINT,
                "{command} 不该被当成错误指令"
            );
        }

        assert_eq!(
            handle_private_command("/nonsense", &[], 1),
            UNKNOWN_COMMAND_REPLY
        );
        assert_eq!(
            handle_private_command("/help", &["多余参数"], 1),
            usage("/help")
        );
    }
}
