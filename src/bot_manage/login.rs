//! `/bot login`：启动一次 FishingBot 登录会话，等它打印微软设备码并写出 refreshToken。
//!
//! 两段式：第一段（[`begin_bot_login`]）启动会话、拿到设备码就返回，让路由层先把链接发给用户；
//! 第二段（[`finish_bot_login`]）等用户授权、把 Token 落盘，停止登录会话，再自动把机器人
//! 加入启动队列。登录等待不会阻塞设备码回复。
//!
//! 登录期间会把旧 Token 改名为 `<token>.renewing`：若 FishingBot 优先读取旧 Token，将不会进入
//! 设备码流程。登录失败/超时会恢复备份；成功则删除备份。

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::config;
use crate::utils::{
    BotDirEntry, StartupQueue, is_bot_screen_running, launch_bot_session, read_last_lines,
    start_bot_sessions, stop_one_bot_session,
};

/// FishingBot 打印设备码时使用的固定短语。
const DEVICE_CODE_MARKER: &str = "using the following code";

/// FishingBot 登录失败时的标记。
const AUTH_FAILED_MARKER: &str = "Error while creating refresh token";

/// 等设备码的最长时间（秒）。
const CODE_WAIT_SECS: u64 = 60;

/// 等用户授权的最长时间（秒），与设备码有效期一致。
const AUTHORIZE_WAIT_SECS: u64 = 600;

/// 轮询间隔（毫秒）。
const POLL_INTERVAL_MS: u64 = 500;

/// 每次轮询读取的日志行数。
const LOG_TAIL_LINES: usize = 200;

/// 旧 Token 备份的后缀。
const REPLACED_TOKEN_SUFFIX: &str = "renewing";

/// 登录第一段的结果：发给用户的设备码 + 第二段要用的会话信息。
#[derive(Debug)]
pub struct BotLoginPrompt {
    /// 登录链接与设备码（直接发到聊天里）。
    pub message: String,
    /// 第二段使用的会话信息。
    session: LoginSession,
}

/// 一次登录会话。
#[derive(Debug)]
struct LoginSession {
    /// 机器人目录条目。
    bot_dir_entry: BotDirEntry,
    /// 本次登录的日志文件。
    log_path: PathBuf,
    /// 旧 Token 的备份路径；没有旧 Token 时为 `None`。
    replaced_token_path: Option<PathBuf>,
}

impl LoginSession {
    /// 机器人名。
    fn bot_name(&self) -> &str {
        &self.bot_dir_entry.bot_name
    }

    /// Token 文件路径。
    fn token_path(&self) -> PathBuf {
        self.bot_dir_entry
            .bot_dir_path
            .join(&config::get().runtime.token_file)
    }
}

/// 第一段：权限检查 → 备份旧 Token → 启动登录会话 → 等设备码。
///
/// 权限在业务层由 [`BotDirEntry::find_for_caller`] 判定（管理员可跨目录）；
/// 已运行、已在启动队列里的一律拒绝，避免两个 JVM 抢同一个账号。
pub fn begin_bot_login(raw_bot_name: &str, caller_qq: i64) -> Result<BotLoginPrompt, String> {
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;
    let bot_name = bot_dir_entry.bot_name.clone();

    if is_bot_screen_running(&bot_name) {
        return Err(format!(
            "机器人 '{bot_name}' 正在运行；如需重新登录，请先执行 /stop {bot_name}。"
        ));
    }
    if StartupQueue::bot_in_queue(&bot_name) {
        return Err(format!(
            "机器人 '{bot_name}' 已在启动队列中；请先执行 /stop {bot_name} 取消排队，再执行 /bot login。"
        ));
    }

    let replaced_token_path = move_existing_token_aside(&bot_dir_entry)?;

    // 没有 Token（或已被移走）时，FishingBot 会自己走微软设备码登录。
    let log_path = match launch_bot_session(&bot_dir_entry) {
        Ok(log_path) => log_path,
        Err(message) => {
            restore_replaced_token(replaced_token_path);
            return Err(message);
        }
    };

    let session = LoginSession {
        bot_dir_entry,
        log_path,
        replaced_token_path,
    };

    let Some((verification_url, user_code)) = wait_for_device_code(&session) else {
        let message = if is_bot_screen_running(session.bot_name()) {
            format!(
                "机器人 '{}' 的登录会话已启动，但 60 秒内未获取设备码。\n请确认 FishingBot 支持微软登录，并使用 /log {} 100 查看输出。",
                session.bot_name(),
                session.bot_name()
            )
        } else {
            format!(
                "机器人 '{}' 的登录会话未能启动（通常为 jar 路径或内存配置问题）。",
                session.bot_name()
            )
        };
        stop_login_session(&session);
        restore_replaced_token(session.replaced_token_path);
        return Err(message);
    };

    Ok(BotLoginPrompt {
        message: format!(
            "机器人 '{bot_name}' 正在登录微软账号。\n1. 打开：{verification_url}\n2. 输入代码：{user_code}\n完成后请等待下一条消息（最长 10 分钟）。"
        ),
        session,
    })
}

/// 第二段：等 Token 落盘 → 停止登录会话 → 自动启动机器人，返回可直接回复的文本。
///
/// 登录失败/超时时恢复旧 Token；登录成功时删除备份。自动启动走启动队列，
/// 因此这里会等队列把这一台启动（和 `/start` 的等待口径一致）。
pub fn finish_bot_login(prompt: BotLoginPrompt) -> String {
    let session = prompt.session;
    let bot_name = session.bot_name().to_string();

    let token_ready = wait_for_token(&session);
    stop_login_session(&session);

    if !token_ready {
        // 新 Token 没写成：把旧凭据放回去，至少让这个账号还能用原来的方式启动。
        let had_replaced_token = session.replaced_token_path.is_some();
        restore_replaced_token(session.replaced_token_path);
        return format!(
            "机器人 '{bot_name}' 登录失败或超时，未生成新 Token。{}",
            if had_replaced_token {
                "旧 Token 已恢复。"
            } else {
                "请重新执行 /bot login。"
            }
        );
    }

    // 新 Token 已经写入：旧备份不再需要。
    remove_replaced_token(session.replaced_token_path);

    // 重新构造条目以刷新 Token 状态（begin 时的条目还是 Missing）。
    let bot_dir_entry = match BotDirEntry::new(&bot_name, session.bot_dir_entry.owner_qq) {
        Ok(bot_dir_entry) => bot_dir_entry,
        Err(message) => {
            return format!("机器人 '{bot_name}' 登录成功，但启动前检查失败：{message}");
        }
    };

    let mut report = start_bot_sessions(&[bot_dir_entry]);
    report.set_title(format!("机器人 '{bot_name}' 登录成功；自动启动结果"));

    if report.is_ok() {
        format!(
            "机器人 '{bot_name}' 登录成功，Token 已保存。\n{}",
            report.render()
        )
    } else {
        format!(
            "机器人 '{bot_name}' 登录成功，Token 已保存，但自动启动失败：\n{}",
            report.render()
        )
    }
}

/// 轮询日志，等到设备码提示；返回 `(登录链接, 设备码)`。
///
/// 会话中途消失就立即返回 `None`（JVM 已经退出，再等也没有意义）。
fn wait_for_device_code(session: &LoginSession) -> Option<(String, String)> {
    let deadline = Instant::now() + Duration::from_secs(CODE_WAIT_SECS);

    while Instant::now() < deadline {
        if let Some(prompt) = read_device_code_prompt(&session.log_path) {
            return Some(prompt);
        }
        if !is_bot_screen_running(session.bot_name()) {
            return None;
        }
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    None
}

/// 从日志尾部解析设备码提示。
fn read_device_code_prompt(log_path: &Path) -> Option<(String, String)> {
    let log_lines = read_last_lines(log_path, LOG_TAIL_LINES).unwrap_or_default();
    parse_device_code_prompt(&log_lines)
}

/// 解析 `using the following code <代码> at <链接>`。
///
/// 日志行形如：
/// `[WARNING] Please log in with your Microsoft account using the following code AQHV33R4 at https://www.microsoft.com/link.`
fn parse_device_code_prompt(log_lines: &[String]) -> Option<(String, String)> {
    for line in log_lines {
        let Some(after_marker) = line.split(DEVICE_CODE_MARKER).nth(1) else {
            continue;
        };

        let mut parts = after_marker.split(" at ");
        let Some(user_code) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(verification_url) = parts.next().map(str::trim) else {
            continue;
        };

        let verification_url = verification_url.trim_end_matches('.').trim();
        if user_code.is_empty() || verification_url.is_empty() {
            continue;
        }

        return Some((verification_url.to_string(), user_code.to_string()));
    }

    None
}

/// 等 Token 文件出现且非空；同时盯着登录失败标记与会话存活。
fn wait_for_token(session: &LoginSession) -> bool {
    let token_path = session.token_path();
    let deadline = Instant::now() + Duration::from_secs(AUTHORIZE_WAIT_SECS);

    while Instant::now() < deadline {
        if token_file_ready(&token_path) {
            // 给 JAR 一点时间把内容写完，再做最后的确认。
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            return token_file_ready(&token_path);
        }

        let log_lines = read_last_lines(&session.log_path, LOG_TAIL_LINES).unwrap_or_default();
        if log_lines
            .iter()
            .any(|line| line.contains(AUTH_FAILED_MARKER))
        {
            return false;
        }
        if !is_bot_screen_running(session.bot_name()) {
            // 会话退出前可能刚好写完 Token，再给文件系统一点时间。
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            return token_file_ready(&token_path);
        }

        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    false
}

/// Token 文件存在且不是空文件。
fn token_file_ready(token_path: &Path) -> bool {
    token_path
        .metadata()
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
}

/// 停止登录会话；会话已经退出时什么也不做。
fn stop_login_session(session: &LoginSession) {
    if is_bot_screen_running(session.bot_name()) {
        let _report = stop_one_bot_session(session.bot_dir_entry.clone());
    }
}

/// 把已有 Token 改名为 `<token>.renewing`，返回备份路径。
fn move_existing_token_aside(bot_dir_entry: &BotDirEntry) -> Result<Option<PathBuf>, String> {
    let token_path = bot_dir_entry
        .bot_dir_path
        .join(&config::get().runtime.token_file);
    if !token_path.exists() {
        return Ok(None);
    }

    let replaced_token_path = replaced_token_path(&token_path);
    // 上一次登录留下的备份先清掉，否则 rename 会覆盖失败或留下两份。
    if replaced_token_path.exists() {
        fs::remove_file(&replaced_token_path)
            .map_err(|e| format!("删除旧的 Token 备份失败：{e}"))?;
    }
    fs::rename(&token_path, &replaced_token_path)
        .map_err(|e| format!("备份旧 Token 失败（登录会话未启动）：{e}"))?;

    Ok(Some(replaced_token_path))
}

/// 备份路径：原文件后加 `.renewing`。
fn replaced_token_path(token_path: &Path) -> PathBuf {
    let mut replaced = token_path.as_os_str().to_os_string();
    replaced.push(format!(".{REPLACED_TOKEN_SUFFIX}"));
    PathBuf::from(replaced)
}

/// 备份路径还原成原 Token 路径。
fn restored_token_path(replaced_token_path: &Path) -> Option<PathBuf> {
    let file_name = replaced_token_path.file_name()?.to_str()?;
    let original_name = file_name.strip_suffix(&format!(".{REPLACED_TOKEN_SUFFIX}"))?;

    Some(replaced_token_path.with_file_name(original_name))
}

/// 登录失败时把备份放回原处（原 Token 已存在时保留新的，不覆盖）。
fn restore_replaced_token(replaced_token_path: Option<PathBuf>) {
    let Some(replaced_token_path) = replaced_token_path else {
        return;
    };
    let Some(token_path) = restored_token_path(&replaced_token_path) else {
        return;
    };
    // 原路径若已有一个空文件（登录中途失败留下的），先删掉再恢复，避免旧 Token 丢失。
    if token_path.exists() {
        let is_empty = token_path
            .metadata()
            .map(|metadata| metadata.len() == 0)
            .unwrap_or(false);
        if !is_empty {
            return;
        }
        let _ = fs::remove_file(&token_path);
    }

    if let Err(e) = fs::rename(&replaced_token_path, &token_path) {
        log::warn!(
            "恢复旧 Token 失败（{} -> {}）: {e}",
            replaced_token_path.display(),
            token_path.display()
        );
    }
}

/// 登录成功后删除备份。
fn remove_replaced_token(replaced_token_path: Option<PathBuf>) {
    let Some(replaced_token_path) = replaced_token_path else {
        return;
    };
    if let Err(e) = fs::remove_file(&replaced_token_path) {
        log::warn!("删除旧 Token 备份失败: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析设备码、网址并去除网址末尾的句点。
    #[test]
    fn device_code_prompt_is_parsed() {
        let lines = vec![
            "[2026-10-07 02:04:46] [INFO   ] Trying to authenticate at Microsoft...".to_string(),
            "[2026-10-07 02:04:48] [WARNING] Please log in with your Microsoft account using the \
             following code AQHV33R4 at https://www.microsoft.com/link."
                .to_string(),
        ];

        assert_eq!(
            parse_device_code_prompt(&lines),
            Some((
                "https://www.microsoft.com/link".to_string(),
                "AQHV33R4".to_string()
            ))
        );
    }

    /// 无关键字或缺少代码/网址时返回 `None`。
    #[test]
    fn unrecognized_prompt_lines_are_skipped() {
        let cases = vec![
            vec!["[INFO   ] Trying to authenticate at Microsoft...".to_string()],
            vec!["using the following code AQHV33R4".to_string()],
            vec!["using the following code  at https://www.microsoft.com/link.".to_string()],
            vec!["using the following code AQHV33R4 at ".to_string()],
            Vec::new(),
        ];

        for lines in cases {
            assert_eq!(parse_device_code_prompt(&lines), None, "{lines:?}");
        }
    }

    /// 备份路径可还原为原 Token 路径。
    #[test]
    fn replaced_token_path_round_trips() {
        let token_path = Path::new("/srv/bots/42/duck/refreshToken");
        let replaced = replaced_token_path(token_path);

        assert_eq!(
            replaced,
            Path::new("/srv/bots/42/duck/refreshToken.renewing")
        );
        assert_eq!(
            restored_token_path(&replaced),
            Some(token_path.to_path_buf())
        );
    }
}
