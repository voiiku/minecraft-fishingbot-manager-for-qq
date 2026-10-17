//! screen 会话的检测与指令注入。
//!
//! screen 是本项目与机器人进程之间的唯一通道：会话存在 = 机器人在运行（见
//! [`crate::utils::running_bots`]），游戏内指令与 Ctrl+C 都靠 `screen -X stuff` 注入。
//!
//! 会话名的唯一来源是 [`session_name_for`]（前缀取自配置 `runtime.screen_prefix`）：它先清洗机器人名，
//! 名字非法就返回 `Err` —— 这种目录不会有我们用同一口径拉起的会话，调用方按"不在运行"处理。
//! 全项目只有这一份 `screen -list` 解析口径，"在不在"与"PID 是多少"不会出现分歧。

use std::collections::HashMap;
use std::process::Command;

use crate::config;

use super::bot_name::ToCleanBotName;

/// 检测指定会话名是否在运行。
///
/// 基于 [`active_screen_sessions`] 的快照：会话名完整匹配，所以 `fb_duck2` 不会被当成 `fb_duck`。
pub fn is_screen_session_running(session_name: &str) -> bool {
    active_screen_sessions().contains_key(session_name)
}

/// 根据机器人名拼出 screen 会话名（形如 `fb_机器人名`）。
///
/// 纯函数：前缀与名字都由调用方给，名字必须是**已清洗**的名字（[`ToCleanBotName::to_clean_bot_name`] 的产物），
/// 所以这条"会话名怎么拼"的口径可以单独测试。
pub fn session_name(clean_bot_name: &str, screen_prefix: &str) -> String {
    format!("{screen_prefix}{clean_bot_name}")
}

/// [`session_name`] 的便利版：清洗原始机器人名 + 只读 `runtime.screen_prefix` 这一个字段。
/// 名字非法（含特殊字符）时返回 `Err`：这种目录名不可能有我们用同一口径拉起的会话。
pub fn session_name_for(raw_bot_name: &str) -> Result<String, String> {
    let clean_bot_name = raw_bot_name.to_clean_bot_name()?;

    Ok(session_name(
        &clean_bot_name,
        &config::get().runtime.screen_prefix,
    ))
}

/// 检测某个机器人对应的会话是否在运行。名字非法时按"没有会话"处理（返回 `false`）。
pub fn is_bot_screen_running(raw_bot_name: &str) -> bool {
    session_name_for(raw_bot_name)
        .is_ok_and(|target_session_name| is_screen_session_running(&target_session_name))
}

/// 把一条游戏内指令注入机器人的会话（`/command` 与 `/commandall` 共用）。
///
/// screen 原语，**自己不做任何校验**：调用方（业务层）负责判权限与"机器人是否在运行"，
/// 返回的错误文案可直接作为回复。
/// 末尾补一个换行符，screen 的伪终端才会把内容作为一次输入提交，否则内容只会停在输入行里。
pub fn send_command_to_single_bot(bot_name: &str, bot_command: &str) -> Result<(), String> {
    let target_session_name = session_name_for(bot_name)?;
    let payload = format!("{bot_command}\n");
    let status = Command::new("screen")
        .args(["-S", &target_session_name, "-X", "stuff", &payload])
        .status()
        .map_err(|e| format!("向机器人 '{bot_name}' 发送指令失败: {e}"))?;

    if !status.success() {
        return Err(format!("向机器人 '{bot_name}' 发送指令失败"));
    }

    Ok(())
}

/// 向会话注入 Ctrl+C，触发 JVM 的 Shutdown Hook 以正常退出。
///
/// 所有停止入口都复用此处，保证停止行为一致。
pub fn send_ctrl_c_to_session(session_name: &str) -> Result<(), String> {
    Command::new("screen")
        .args(["-X", "-S", session_name, "stuff", "^C"])
        .status()
        .map_err(|e| format!("发送停止信号失败: {e}"))?;
    Ok(())
}

/// 强制退出会话，用于正常退出超时后的兜底。
pub fn force_quit_session(session_name: &str) -> Result<(), String> {
    Command::new("screen")
        .args(["-X", "-S", session_name, "quit"])
        .status()
        .map_err(|e| format!("强制终止机器人失败: {e}"))?;
    Ok(())
}

/// 解析 `screen -list` 的输出，得到「会话名 -> PID」。
///
/// 输出形如 `12345.fb_slancelenk    (Detached)`，只认「PID.会话名」这种 token：
/// 头部说明行、尾部 Socket 统计行与空行都跳过；点号后的部分整体是会话名，名字里带点也不影响。
///
/// 不特判死会话：机器人就是"screen 会话里跑一个 JVM"，JVM 一死窗口就关、会话自己注销；
/// 万一 screen 服务进程被 SIGKILL（例如整个 cgroup 被强杀）留下 `(Dead ???)` 残壳，
/// 它也照样算作「运行中」—— 这样 `/stop` 才会走到 `screen -X quit` 把残壳清掉。
fn parse_screen_list(output: &str) -> HashMap<String, u32> {
    let mut sessions = HashMap::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some(session_part) = trimmed.split_whitespace().next() else {
            continue;
        };
        let Some(dot_pos) = session_part.find('.') else {
            continue;
        };
        let Ok(pid) = session_part[..dot_pos].parse::<u32>() else {
            continue;
        };

        sessions.insert(session_part[dot_pos + 1..].to_string(), pid);
    }

    sessions
}

/// 抓取当前所有存活的 screen 会话及其 PID，一次外部命令完成。
pub fn active_screen_sessions() -> HashMap<String, u32> {
    let Ok(output) = Command::new("screen").arg("-list").output() else {
        return HashMap::new();
    };

    parse_screen_list(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 会话名 = 配置前缀 + 已清洗的机器人名（大小写与非法字符在 [`ToCleanBotName::to_clean_bot_name`] 那一步就定了）。
    #[test]
    fn session_names_use_the_prefix() {
        assert_eq!(session_name("duck", "fb_"), "fb_duck");
        assert_eq!(session_name("duck", ""), "duck");
    }

    /// 典型的 `screen -list` 输出：头部说明行 + 两个会话 + 尾部 Socket 统计行。
    #[test]
    fn parses_sessions_with_pid() {
        let output = "There are screens on:\n\
                      \t12345.fb_slancelenk\t(Detached)\n\
                      \t678.fb_duck\t(Detached)\n\
                      2 Sockets in /run/screen/S-fishbot.\n";

        let sessions = parse_screen_list(output);

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions.get("fb_slancelenk"), Some(&12345));
        assert_eq!(sessions.get("fb_duck"), Some(&678));
    }

    /// 头部说明行、尾部统计行与空行都不该被当成会话。
    #[test]
    fn notes_and_blank_lines_are_ignored() {
        let output = "There is a screen on:\n\
                      No Sockets found in /run/screen/S-fishbot.\n\
                      \n";

        let sessions = parse_screen_list(output);
        assert!(
            sessions.is_empty(),
            "这些行都不该被当成会话: 输入={output:?} 解析结果={sessions:?}"
        );
    }

    /// 会话名里的点不是分隔符：只有第一个点之前的 PID 参与解析。
    #[test]
    fn dots_inside_session_name_are_kept() {
        let sessions = parse_screen_list("\t7.fb.v2_duck\t(Detached)\n");

        assert_eq!(sessions.get("fb.v2_duck"), Some(&7));
    }

    /// 会话名做完整匹配：`fb_duck2` 不能被当成 `fb_duck` 在运行。
    #[test]
    fn session_name_matches_exactly() {
        let sessions = parse_screen_list("\t1.fb_duck2\t(Detached)\n");

        assert!(!sessions.contains_key("fb_duck"), "前缀相同不算同名会话");
        assert!(sessions.contains_key("fb_duck2"));
    }
}
