//! kovi 插件入口：解析 QQ 消息里的指令、把参数原样切分好，再交由 `mc_qq_fishbot` 库执行。
//!
//! 只负责"认指令 → 洗参数 → 分发"：群聊走 `/xxx` 指令，私聊只放行 `/help`、`/token` 与 `/log`；
//! **不查权限** —— 批量指令的权限在库的路由层（`handle_all_bots_control`）判一次，
//! 单机器人指令的权限在业务层按目录归属判定（见 `src/lib.rs` 的模块文档）。
//! 业务动作位于库中，便于独立阅读与测试。
//!
//! **本层只切分，不清洗**：消息按空白切成片段后原样传递，由实际使用参数的层清洗一次
//! （机器人名见 `ToCleanBotName::to_clean_bot_name`，游戏内指令片段见
//! `bot_control::command::join_bot_command`）；参数名以 `raw_` / `clean_` 标明形态，
//! 避免"到底收到的是原始还是清洗过的"这种疑问。
//! 数字例外：`/log` 的行数、`/ping` 的端口在本文件就解析成数字 —— 解析失败要立即回用法提示。
//!
//! 心跳事件（[`HeartbeatEvent`]）在 [`crate::heartbeat_event`] 里单独实现，本文件只负责注册它
//! 并启动库里的心跳看门狗。
//!
//! 插件入口在一个进程里只会被框架调用一次（本程序不重建 Bot：连接一断即退出，
//! 交给 systemd 重新启动），因此这里不需要幂等守卫。

use kovi::PluginBuilder;
use kovi_onebot::{EventRegistrar, PrivateMsgEvent};
use mc_qq_fishbot::{
    AllBotsControlAction, BotControlAction, BotManageAction, ShowAction, StartupQueue,
    handle_all_bots_control, handle_bot_control, handle_bot_log, handle_bot_login_begin,
    handle_bot_login_finish, handle_bot_manage, handle_checkserver, handle_help, handle_ping,
    handle_show, handle_startup_receipt, handle_status, handle_whoami, note_heartbeat,
    watch_heartbeat,
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

        // 启动请求可能长时间排队：批量指令先返回进度回执；单机指令仅在队列非空时返回回执。
        let is_batch_start = matches!(command, "/startall" | "/restartall") && args.is_empty();
        let is_busy_single_start = matches!(command, "/start" | "/restart")
            && args.len() == 1
            && StartupQueue::pending_bot_count() > 0;
        if (is_batch_start || is_busy_single_start)
            && let Some(receipt) = handle_startup_receipt(command, event.user_id)
        {
            event.reply(receipt);
        }

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

        // `/bot login` 要分两段回复（设备码 → 登录结果），所以单独走一条分支。
        if command == "/bot" && args.first() == Some(&"login") {
            handle_private_login(&event, &args, event.user_id).await;
            return;
        }

        let command = command.to_string();
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let caller_qq = event.user_id;
        let reply = run_blocking(move || {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            handle_private_command(&command, &args, caller_qq)
        })
        .await;
        event.reply(reply);
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
/// 群聊与私聊入口共用，确保两条入口的指令识别规则（去空白、按空白切分）保持一致。
fn parse_command(text: &str) -> Option<(&str, Vec<&str>)> {
    let mut parts = text.split_whitespace();
    let command = parts.next()?;

    command.starts_with('/').then(|| (command, parts.collect()))
}

/// 构造用法错误提示。
fn usage(example: &str) -> String {
    format!("用法：{example}")
}

fn parse_ping_port(port: Option<&str>) -> Result<Option<u16>, String> {
    port.map(|port| {
        port.parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| "端口必须为 1 到 65535 的整数。".to_string())
    })
    .transpose()
}

/// 私聊 `/bot login`：分两段回复，设备码先发，登录结果后发。
///
/// 两段都跑在阻塞线程池里：第一段要等设备码（最长 60 秒），第二段要等授权（最长 10 分钟）。
async fn handle_private_login(event: &PrivateMsgEvent, args: &[&str], caller_qq: i64) {
    if args.len() != 2 {
        event.reply(usage("/bot login <机器人名>"));
        return;
    }

    let raw_bot_name = args[1].to_string();
    event.reply("正在启动登录会话，等待微软设备码（最长 60 秒）。");

    let begin_result =
        tokio::task::spawn_blocking(move || handle_bot_login_begin(&raw_bot_name, caller_qq)).await;

    let prompt = match begin_result {
        Ok(Ok(prompt)) => prompt,
        Ok(Err(message)) => {
            event.reply(message);
            return;
        }
        Err(error) => {
            event.reply(format!("登录流程出错：{error}"));
            return;
        }
    };

    // 先把设备码发出去，再进入最长 10 分钟的等待。
    event.reply(prompt.message.clone());

    match tokio::task::spawn_blocking(move || handle_bot_login_finish(prompt)).await {
        Ok(reply) => event.reply(reply),
        Err(error) => event.reply(format!("登录流程出错：{error}")),
    }
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
        "/whoami" => {
            if args.is_empty() {
                handle_whoami(caller_qq)
            } else {
                usage("/whoami")
            }
        }
        "/status" => {
            if args.is_empty() {
                run_blocking(handle_status).await
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
                let port = match parse_ping_port(args.get(1).copied()) {
                    Ok(port) => port,
                    Err(message) => return Some(message),
                };
                handle_ping(host, port).await
            }
        }

        // 单个机器人的控制指令。
        "/start" => bot_control_reply(BotControlAction::Start, command, args, caller_qq).await,
        "/stop" => bot_control_reply(BotControlAction::Stop, command, args, caller_qq).await,
        "/restart" => bot_control_reply(BotControlAction::Restart, command, args, caller_qq).await,
        "/pause" => bot_control_reply(BotControlAction::Pause, command, args, caller_qq).await,
        "/unpause" => bot_control_reply(BotControlAction::Unpause, command, args, caller_qq).await,
        // 向机器人发送游戏内指令：只切分，不拼接、不清洗 —— 原始片段在业务函数里拼一次。
        "/command" => {
            if args.len() < 2 {
                usage("/command <机器人名> <指令内容>")
            } else {
                let raw_bot_name = args[0].to_string();
                let raw_bot_command_args: Vec<String> =
                    args[1..].iter().map(|arg| arg.to_string()).collect();
                run_blocking(move || {
                    let command_arg_refs: Vec<&str> =
                        raw_bot_command_args.iter().map(String::as_str).collect();
                    handle_bot_control(
                        BotControlAction::Command(&command_arg_refs),
                        &raw_bot_name,
                        caller_qq,
                    )
                })
                .await
            }
        }
        // 机器人增删与改名。
        "/bot" => {
            if args.first() == Some(&"login") {
                "该指令仅支持私聊。请私聊机器人发送 /bot login <机器人名>。".to_string()
            } else {
                handle_bot_manage_args_async(args, caller_qq).await
            }
        }
        // 信息查询。
        "/show" => {
            if args.len() != 1 {
                usage("/show <status|botconfig|mybots|allbots|admins>")
            } else {
                match ShowAction::from_name(args[0]) {
                    Some(action) => run_blocking(move || handle_show(action, caller_qq)).await,
                    None => usage("/show <status|botconfig|mybots|allbots|admins>"),
                }
            }
        }
        // 全服批量指令（仅管理员）：权限由库的路由层 handle_all_bots_control 校验，这里只切参数。
        "/startall" => {
            all_bots_control_reply(AllBotsControlAction::StartAll, command, args, caller_qq).await
        }
        "/stopall" => {
            all_bots_control_reply(AllBotsControlAction::StopAll, command, args, caller_qq).await
        }
        "/restartall" => {
            all_bots_control_reply(AllBotsControlAction::RestartAll, command, args, caller_qq).await
        }
        "/commandall" => {
            if args.is_empty() {
                usage("/commandall <指令内容>")
            } else {
                let raw_bot_command_args: Vec<String> =
                    args.iter().map(|arg| arg.to_string()).collect();
                run_blocking(move || {
                    let command_arg_refs: Vec<&str> =
                        raw_bot_command_args.iter().map(String::as_str).collect();
                    handle_all_bots_control(
                        AllBotsControlAction::CommandAll(&command_arg_refs),
                        caller_qq,
                    )
                })
                .await
            }
        }
        _ => return None,
    };

    Some(reply)
}

/// 在阻塞线程池里跑一段业务：起 JVM、等队列、读日志都不该占住 async worker。
async fn run_blocking<F>(work: F) -> String
where
    F: FnOnce() -> String + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(reply) => reply,
        Err(error) => format!("后台任务执行失败：{error}"),
    }
}

/// 私聊统一提示：仅允许以下指令。
const PRIVATE_ONLY_HINT: &str =
    "该指令仅支持群聊。私聊可用：/help、/whoami、/bot login、/token、/log。";

/// 完全不认识的指令统一回这一句（不区分群里还是私聊的用户输入）。
const UNKNOWN_COMMAND_REPLY: &str = "未知指令。发送 /help 查看可用指令。";

/// 私聊指令分发：只放行 `/help`、`/whoami`、`/bot login`、`/token`、`/log`，
/// 其余按"已识别但不可用"与"未知指令"分别返回提示。
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
        "/whoami" => {
            if args.is_empty() {
                handle_whoami(caller_qq)
            } else {
                usage("/whoami")
            }
        }
        "/token" => {
            if args.len() != 2 {
                usage("/token <机器人名> <Token 字符串>")
            } else {
                // Token 作为单个参数原样透传（切分时已去除首尾空白），写入前不再修改。
                handle_bot_manage(
                    BotManageAction::Token(args[1].to_string()),
                    args[0],
                    caller_qq,
                )
            }
        }
        "/log" => {
            if !(1..=2).contains(&args.len()) {
                usage("/log <机器人名> [日志行数]")
            } else {
                match args.get(1).map_or(Ok(100), |lines| lines.parse::<usize>()) {
                    Ok(lines) => handle_bot_log(caller_qq, args[0], lines),
                    Err(_) => usage("/log <机器人名> [日志行数]"),
                }
            }
        }
        // `/bot login` 由私聊入口单独处理（要分两段回复）；其余 `/bot` 子命令只在群聊提供。
        "/bot" => {
            if args.first() == Some(&"login") {
                usage("/bot login <机器人名>")
            } else {
                PRIVATE_ONLY_HINT.to_string()
            }
        }
        // 能识别、但只在群聊提供：Token 与日志属于敏感内容，其余指令在群里发。
        "/status" | "/checkserver" | "/ping" | "/start" | "/stop" | "/restart" | "/pause"
        | "/unpause" | "/command" | "/show" | "/startall" | "/stopall" | "/restartall"
        | "/commandall" => PRIVATE_ONLY_HINT.to_string(),
        // 未知指令：仅返回继续操作的提示，不推断用户意图。
        _ => UNKNOWN_COMMAND_REPLY.to_string(),
    }
}

/// 解析 `/bot` 子命令：add / del / rename。
fn handle_bot_manage_args(args: &[&str], caller_qq: i64) -> String {
    let Some(action) = args.first() else {
        return usage("/bot <add|del|rename|login> <机器人名>");
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
        _ => usage("/bot <add|del|rename|login> <机器人名>"),
    }
}

/// 单机器人控制指令的公共部分：这类指令只带一个机器人名（权限在业务层按目录归属判定，本层不查）。
///
/// 放进阻塞线程池：`/start`、`/stop` 要等启动队列与 `screen` 会话，不能占住 async worker。
async fn bot_control_reply(
    action: BotControlAction<'static>,
    command: &str,
    args: &[&str],
    caller_qq: i64,
) -> String {
    if args.len() != 1 {
        return usage(&format!("{command} <机器人名>"));
    }

    let raw_bot_name = args[0].to_string();
    run_blocking(move || handle_bot_control(action, &raw_bot_name, caller_qq)).await
}

/// `/bot add|del|rename`：参数转成 owned 后放进阻塞线程池（`rename` 会先停会话）。
async fn handle_bot_manage_args_async(args: &[&str], caller_qq: i64) -> String {
    let owned_args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();

    run_blocking(move || {
        let arg_refs: Vec<&str> = owned_args.iter().map(String::as_str).collect();
        handle_bot_manage_args(&arg_refs, caller_qq)
    })
    .await
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
async fn all_bots_control_reply(
    action: AllBotsControlAction<'static>,
    command: &str,
    args: &[&str],
    caller_qq: i64,
) -> String {
    if !args.is_empty() {
        return usage(command);
    }

    run_blocking(move || handle_all_bots_control(action, caller_qq)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_rejects_invalid_explicit_ports() {
        assert_eq!(parse_ping_port(None).unwrap(), None);
        assert_eq!(parse_ping_port(Some("25565")).unwrap(), Some(25565));
        for port in ["0", "-1", "65536", "abc", ""] {
            assert!(parse_ping_port(Some(port)).is_err(), "{port}");
        }
    }

    /// "仅群聊可用"与"未知指令"应分别返回对应提示。
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
