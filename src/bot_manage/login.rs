//! `/bot login`：启动一次 FishingBot 登录会话，等它打印微软设备码并写出 refreshToken。
//!
//! 两段式：第一段（[`begin_bot_login`]）启动会话、拿到设备码就返回，让路由层先把链接发给用户；
//! 第二段（[`finish_bot_login`]）等用户授权、把 Token 落盘，保留已经完成登录的 JVM。
//! 登录等待不会阻塞设备码回复。
//!
//! 登录期间会把旧 Token 改名为 `<token>.renewing`：若 FishingBot 优先读取旧 Token，将不会进入
//! 设备码流程。登录失败/超时会恢复备份；成功则删除备份。

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::config;
use crate::utils::{
    BotDirEntry, StartupQueue, active_screen_sessions, is_bot_screen_running, launch_bot_session,
    read_last_lines, session_name_for, stop_one_bot_session_if_pid, token_file_has_content,
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
    /// 防止登录等待期间同名的新会话被误停止。
    session_pid: Option<u32>,
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

    fn is_running(&self) -> bool {
        self.session_pid.is_some() && self.session_pid == bot_session_pid(self.bot_name())
    }
}

fn bot_session_pid(bot_name: &str) -> Option<u32> {
    let name = session_name_for(bot_name).ok()?;
    active_screen_sessions().get(&name).copied()
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
            if let Err(rollback) = restore_replaced_token(replaced_token_path) {
                return Err(format!("{message}\n{rollback}"));
            }
            return Err(message);
        }
    };

    let session = LoginSession {
        session_pid: bot_session_pid(&bot_name),
        bot_dir_entry,
        log_path,
        replaced_token_path,
    };

    let Some((verification_url, user_code)) = wait_for_device_code(&session) else {
        let failed = read_last_lines(&session.log_path, LOG_TAIL_LINES)
            .unwrap_or_default()
            .iter()
            .any(|line| authentication_failed(line));
        let message = if failed {
            format!(
                "机器人 '{}' 的微软身份验证失败，请重新执行 /bot login。",
                session.bot_name()
            )
        } else if session.is_running() {
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
        stop_login_session(&session)?;
        restore_replaced_token(session.replaced_token_path)?;
        return Err(message);
    };

    Ok(BotLoginPrompt {
        message: format!(
            "机器人 '{bot_name}' 正在登录微软账号。\n1. 打开：{verification_url}\n2. 输入代码：{user_code}\n完成后请等待下一条消息（最长 10 分钟）。"
        ),
        session,
    })
}

/// 第二段：等 Token 落盘，成功时保留会话，返回可直接回复的文本。
///
/// 只在登录失败/超时时停止会话并恢复旧 Token；登录成功时删除备份、收紧权限。
pub fn finish_bot_login(prompt: BotLoginPrompt) -> String {
    let session = prompt.session;
    let bot_name = session.bot_name().to_string();

    let outcome = wait_for_token(&session);
    if outcome == TokenWaitOutcome::Ready {
        return finish_authenticated_login(&session);
    }
    if let Err(message) = stop_login_session(&session) {
        return message;
    }
    // 授权可能恰好在截止时间或停止过程中完成，不能把刚写出的有效凭据覆盖回旧值。
    if token_file_ready(&session.token_path()) {
        return finish_authenticated_login(&session);
    }

    let had_replaced_token = session.replaced_token_path.is_some();
    if let Err(message) = restore_replaced_token(session.replaced_token_path) {
        return format!("机器人 '{bot_name}' 登录失败或超时。\n{message}");
    }
    format!(
        "机器人 '{bot_name}' 登录失败或超时，未生成新 Token。{}",
        if had_replaced_token {
            "旧 Token 已恢复。"
        } else {
            "请重新执行 /bot login。"
        }
    )
}

fn finish_authenticated_login(session: &LoginSession) -> String {
    let current_pid = bot_session_pid(session.bot_name());
    if current_pid.is_some() && current_pid != session.session_pid {
        return format!(
            "机器人 '{}' 的登录会话已被其他会话替换；Token 备份已保留。",
            session.bot_name()
        );
    }
    tighten_token_permissions(&session.token_path());
    remove_replaced_token(session.replaced_token_path.clone());
    format!("机器人 '{}' 登录成功，Token 已保存。", session.bot_name())
}

fn tighten_token_permissions(token_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(token_path, fs::Permissions::from_mode(0o600)) {
            log::warn!("设置登录 Token 文件 {} 权限失败：{e}", token_path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = token_path;
}

/// 轮询日志，等到设备码提示；返回 `(登录链接, 设备码)`。
///
/// 会话中途消失就立即返回 `None`（JVM 已经退出，再等也没有意义）。
fn wait_for_device_code(session: &LoginSession) -> Option<(String, String)> {
    let deadline = Instant::now() + Duration::from_secs(CODE_WAIT_SECS);

    while Instant::now() < deadline {
        let lines = read_last_lines(&session.log_path, LOG_TAIL_LINES).unwrap_or_default();
        if lines.iter().any(|line| authentication_failed(line)) {
            return None;
        }
        if let Some(prompt) = parse_device_code_prompt(&lines) {
            return Some(prompt);
        }
        if !session.is_running() {
            return read_device_code_prompt(&session.log_path);
        }
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    read_device_code_prompt(&session.log_path)
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
    for line in log_lines.iter().rev() {
        if let Some(after_marker) = line.split_once(DEVICE_CODE_MARKER).map(|(_, rest)| rest)
            && let Some((user_code, url)) = after_marker.split_once(" at ")
            && let Some(url) = verification_url_in(url)
            && !user_code.trim().is_empty()
        {
            return Some((url, user_code.trim().to_string()));
        }
        for marker in [
            "以下验证码",
            "以下代码",
            "设备代码",
            "设备码",
            "验证码",
            "代码",
        ] {
            let Some((_, rest)) = line.split_once(marker) else {
                continue;
            };
            let code =
                rest.trim_start_matches(|c: char| c.is_whitespace() || "：:=，,".contains(c));
            let code: String = code
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if !code.is_empty()
                && let Some(url) = verification_url_in(line)
            {
                return Some((url, code));
            }
        }
    }

    None
}

fn verification_url_in(text: &str) -> Option<String> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let url = text[start..]
        .split(|c: char| c.is_whitespace() || "。；，、（）<>\"'".contains(c))
        .next()?
        .trim_end_matches(['.', ',', ';', ')', ']']);
    let host = url.split_once("://")?.1;
    (!host.is_empty()).then(|| url.to_string())
}

fn authentication_failed(line: &str) -> bool {
    [
        AUTH_FAILED_MARKER,
        "Error while waiting for access token",
        "Authentication failed",
        "Authorization failed",
        "创建刷新令牌时出错",
        "创建刷新令牌失败",
        "获取刷新令牌失败",
        "创建 refresh token 时出错",
        "身份验证失败",
        "授权失败",
    ]
    .iter()
    .any(|marker| line.contains(marker))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenWaitOutcome {
    Ready,
    Failed,
    TimedOut,
}

/// 等 Token 文件出现且非空；同时盯着登录失败标记与会话存活。
fn wait_for_token(session: &LoginSession) -> TokenWaitOutcome {
    let token_path = session.token_path();
    let deadline = Instant::now() + Duration::from_secs(AUTHORIZE_WAIT_SECS);
    wait_for_token_until(session, &token_path, deadline)
}

fn wait_for_token_until(
    session: &LoginSession,
    token_path: &Path,
    deadline: Instant,
) -> TokenWaitOutcome {
    while Instant::now() < deadline {
        let log_lines = read_last_lines(&session.log_path, LOG_TAIL_LINES).unwrap_or_default();
        if log_lines.iter().any(|line| authentication_failed(line)) {
            return TokenWaitOutcome::Failed;
        }
        if token_file_ready(token_path) {
            // 给 JAR 一点时间把内容写完，再做最后的确认。
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            return if token_file_ready(token_path) {
                TokenWaitOutcome::Ready
            } else {
                TokenWaitOutcome::Failed
            };
        }

        if !session.is_running() {
            // 会话退出前可能刚好写完 Token，再给文件系统一点时间。
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            return if token_file_ready(token_path) {
                TokenWaitOutcome::Ready
            } else {
                TokenWaitOutcome::Failed
            };
        }

        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    if token_file_ready(token_path) {
        TokenWaitOutcome::Ready
    } else {
        TokenWaitOutcome::TimedOut
    }
}

/// Token 文件存在且不是空文件。
fn token_file_ready(token_path: &Path) -> bool {
    token_file_has_content(token_path)
}

/// 停止登录会话；会话已经退出时什么也不做。
fn stop_login_session(session: &LoginSession) -> Result<(), String> {
    let current_pid = bot_session_pid(session.bot_name());
    if current_pid.is_some() && current_pid != session.session_pid {
        return Err(format!(
            "机器人 '{}' 的登录会话已被其他会话替换；未停止新会话，Token 备份已保留。",
            session.bot_name()
        ));
    }
    if session.is_running()
        && let Some(pid) = session.session_pid
    {
        let _report = stop_one_bot_session_if_pid(session.bot_dir_entry.clone(), pid);
    }
    if is_bot_screen_running(session.bot_name()) {
        return Err(format!(
            "机器人 '{}' 的登录会话未能停止；Token 备份已保留，请先 /stop 后重试。",
            session.bot_name()
        ));
    }
    Ok(())
}

/// 把已有 Token 改名为 `<token>.renewing`，返回备份路径。
fn move_existing_token_aside(bot_dir_entry: &BotDirEntry) -> Result<Option<PathBuf>, String> {
    let token_path = bot_dir_entry
        .bot_dir_path
        .join(&config::get().runtime.token_file);
    move_existing_token_path_aside(&token_path)
}

fn move_existing_token_path_aside(token_path: &Path) -> Result<Option<PathBuf>, String> {
    let replaced_token_path = replaced_token_path(token_path);
    // 上次登录被中断、只剩备份时先恢复，不能丢掉唯一有效凭据。
    if replaced_token_path.exists() && !token_file_ready(token_path) {
        if token_path.exists() {
            fs::remove_file(token_path).map_err(|e| format!("清理未完成的 Token 失败：{e}"))?;
        }
        fs::rename(&replaced_token_path, token_path)
            .map_err(|e| format!("恢复上次登录的 Token 备份失败：{e}"))?;
    }
    if !token_path.exists() {
        return Ok(None);
    }
    // 上一次登录留下的备份先清掉，否则 rename 会覆盖失败或留下两份。
    if replaced_token_path.exists() {
        fs::remove_file(&replaced_token_path)
            .map_err(|e| format!("删除旧的 Token 备份失败：{e}"))?;
    }
    fs::rename(token_path, &replaced_token_path)
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

/// 登录失败时恢复旧凭据，包括覆盖 JAR 失败前可能留下的不完整文件。
fn restore_replaced_token(replaced_token_path: Option<PathBuf>) -> Result<(), String> {
    let Some(replaced_token_path) = replaced_token_path else {
        return Ok(());
    };
    let Some(token_path) = restored_token_path(&replaced_token_path) else {
        return Err("Token 备份路径无效，未能恢复旧 Token。".to_string());
    };
    if let Err(e) = fs::rename(&replaced_token_path, &token_path) {
        log::warn!(
            "恢复旧 Token 失败（{} -> {}）: {e}",
            replaced_token_path.display(),
            token_path.display()
        );
        return Err(
            "恢复旧 Token 失败；备份仍保存在 .renewing 文件中，请检查文件权限后重试。".to_string(),
        );
    }
    Ok(())
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

    #[test]
    fn token_is_rechecked_when_authorization_deadline_expires() {
        let dir = PathBuf::from("target").join(format!("login-deadline-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let token = dir.join("refreshToken");
        let session = LoginSession {
            bot_dir_entry: BotDirEntry {
                owner_qq: 42,
                bot_name: "duck".to_string(),
                bot_dir_path: dir.clone(),
                bot_token_status: crate::utils::BotTokenStatus::Missing,
            },
            log_path: dir.join("session.log"),
            session_pid: None,
            replaced_token_path: None,
        };
        assert_eq!(
            wait_for_token_until(&session, &token, Instant::now()),
            TokenWaitOutcome::TimedOut
        );
        fs::write(&token, "new token").unwrap();
        assert_eq!(
            wait_for_token_until(&session, &token, Instant::now()),
            TokenWaitOutcome::Ready
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retry_recovers_stranded_backup_before_starting_a_new_login() {
        let dir = PathBuf::from("target").join(format!("login-stranded-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let token = dir.join("refreshToken");
        let backup = replaced_token_path(&token);
        fs::write(&backup, "previous token").unwrap();
        assert_eq!(
            move_existing_token_path_aside(&token).unwrap(),
            Some(backup.clone())
        );
        assert!(!token.exists());
        assert_eq!(fs::read_to_string(&backup).unwrap(), "previous token");
        restore_replaced_token(Some(backup.clone())).unwrap();
        assert_eq!(fs::read_to_string(&token).unwrap(), "previous token");
        fs::write(&backup, "stale backup").unwrap();
        fs::write(&token, "current token").unwrap();
        move_existing_token_path_aside(&token).unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), "current token");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn localized_prompts_and_latest_code_are_parsed() {
        for line in [
            "请使用以下代码 AQHV33R4 在 https://www.microsoft.com/link。登录微软账号。",
            "请在 https://www.microsoft.com/link 输入代码：AQHV33R4。",
            "请使用微软账户登录，验证码：AQHV33R4，网址：https://www.microsoft.com/link。",
        ] {
            assert_eq!(
                parse_device_code_prompt(&[line.to_string()]),
                Some((
                    "https://www.microsoft.com/link".to_string(),
                    "AQHV33R4".to_string()
                )),
                "{line}"
            );
        }
        let lines = [
            "using the following code OLD at https://www.microsoft.com/link.",
            "using the following code NEW at https://www.microsoft.com/link.",
        ]
        .map(str::to_string);
        assert_eq!(parse_device_code_prompt(&lines).unwrap().1, "NEW");
    }

    #[test]
    fn localized_authentication_failures_are_detected() {
        for line in [
            "Error while creating refresh token... Please try again",
            "Error while waiting for access token: denied",
            "创建刷新令牌失败，请重试",
            "微软身份验证失败",
            "授权失败",
        ] {
            assert!(authentication_failed(line), "{line}");
        }
        assert!(!authentication_failed(
            "Trying to authenticate at Microsoft..."
        ));
    }

    #[test]
    fn failed_login_restores_backup_over_incomplete_token() {
        let dir = PathBuf::from("target").join(format!("login-rollback-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let token = dir.join("refreshToken");
        let backup = replaced_token_path(&token);
        fs::write(&token, "partial token").unwrap();
        fs::write(&backup, "previous token").unwrap();
        restore_replaced_token(Some(backup.clone())).unwrap();
        assert_eq!(fs::read_to_string(&token).unwrap(), "previous token");
        assert!(!backup.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&token, fs::Permissions::from_mode(0o644)).unwrap();
            tighten_token_permissions(&token);
            assert_eq!(
                fs::metadata(&token).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&token, " \n\t").unwrap();
        assert!(!token_file_ready(&token));
        fs::remove_dir_all(dir).unwrap();
    }

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
