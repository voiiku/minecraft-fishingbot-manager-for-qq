//! 停止会话：`/stop`、`/stopall`、进程退出收尾，以及重启流程的停止阶段
//! （[`crate::utils::restart_bot_sessions`] 直接复用这里的 [`stop_sessions`]）。
//!
//! 流程固定为：发送 Ctrl+C → 等 JVM 自己退出 → 强杀未退出的 → 复核一次。单机与批量同一段代码，
//! 耗时是常数级、不随机器人数量增长，因此不会超出 systemd 的 `TimeoutStopSec`。
//!
//! 必须让会话正常退出的原因：机器人占着游戏账号时 FishingBot 的自动重连会与真人玩家登录冲突。
//! 日志不需要在这里处理：每个会话的日志文件在拉起会话之前就建好了
//! （[`super::bot_log_files::create_bot_log_file`]），会话一直在往里追加，见 [`super::startup`]。
//!
//! 这是"第一层"收尾；主进程被强杀或卡死时由 systemd 按 `KillMode=control-group` 收掉整个 cgroup
//! （先 SIGTERM、超时才 SIGKILL）兜底，保证不留占着账号的孤儿 JVM。
use std::thread;
use std::time::Duration;

use crate::config;

use super::bot_dirs::{BotDirEntry, all_bot_dir_entries};
use super::report::Report;
use super::running_bots::split_bot_entries_by_running;
use super::screen::{
    active_screen_sessions, force_quit_session, send_ctrl_c_to_session, session_name_for,
};

/// 停止一批会话时要等多久（毫秒）：先等 JVM 优雅退出，超时强杀，再等旧会话注销。
///
/// 与 [`super::startup::LaunchTiming`] 对称：把"两个毫秒数"收成一个具名参数，
/// 调用处不必记住哪个是哪个，`stop_sessions` 里也不再自己读配置。
/// ?停止只有一套参数（单机与批量都读 `batch_stop_grace_ms` / `batch_stop_settle_ms`），
/// ?字段名却带 `batch_` 前缀：要么给单机单独配一套，要么把配置项改成 `stop_grace_ms` 这类中性名字
#[derive(Debug, Clone, Copy)]
pub struct StopTiming {
    /// 发完 Ctrl+C 之后，等 JVM 自己退出的时间；超过就强杀。
    pub force_after_ms: u64,
    /// 强杀之后、复核之前的小缓冲，让旧会话彻底注销。
    pub settle_ms: u64,
}

impl StopTiming {
    /// 从配置取停止流程的等待参数（只读 `timing.batch_stop_grace_ms` 与 `batch_stop_settle_ms`）。
    pub fn from_config() -> Self {
        let timing = &config::get().timing;
        Self {
            force_after_ms: timing.batch_stop_grace_ms,
            settle_ms: timing.batch_stop_settle_ms,
        }
    }
}

/// 停止过程的内部明细，供启动管道判断哪些可以重新拉起。
#[derive(Debug, Default)]
pub(crate) struct StopOutcome {
    /// 停止前正在运行的机器人数量。
    pub(crate) running_bot_count: usize,
    /// 确认已停止的机器人名。
    pub(crate) stopped_bot_names: Vec<String>,
    /// 靠强制终止才停下的机器人名。
    pub(crate) force_stopped_bot_names: Vec<String>,
    /// 停止后仍在运行的机器人名。
    pub(crate) still_running_bot_names: Vec<String>,
    /// 扫描到但原本未运行的机器人。
    pub(crate) not_running_bot_entries: Vec<BotDirEntry>,
    /// 执行过程中的错误信息。
    pub(crate) failure_lines: Vec<String>,
}

/// 停止全部运行中的机器人，包含 Token 已失效的孤儿进程。
///
/// 用于 `/stopall` 与进程退出收尾。返回的报告已填好计数与明细，标题由调用方设置。
pub fn stop_all_bot_sessions() -> Result<Report, String> {
    let (running_bot_entries, not_running_bot_entries) =
        split_bot_entries_by_running(&all_bot_dir_entries()?);
    Ok(stop_report(stop_sessions(
        &running_bot_entries,
        not_running_bot_entries,
        StopTiming::from_config(),
    )))
}

/// 停止单个机器人对应的会话（`/stop` 用）。
///
/// 调用方负责先确认该会话确实在运行，并把已鉴权的条目传进来。
pub fn stop_one_bot_session(bot_dir_entry: BotDirEntry) -> Report {
    stop_report(stop_sessions(
        &[bot_dir_entry],
        Vec::new(),
        StopTiming::from_config(),
    ))
}

/// 停止一批会话：发送 Ctrl+C 等待 JVM 正常退出，超时强杀，最后复核一次。
///
/// 这是所有停止入口的唯一实现（`/stop`、`/stopall`、重启的停止阶段、进程收尾）。
/// `entries_to_stop` 是要停的那批；`not_running_entries` 只是顺带带进报告
/// （算「原本未运行，无需停止」），不参与任何停止动作 —— 传它进来是为了让报告里的
/// 「扫描机器人目录」总数说得通。等待时间由 [`StopTiming`] 给，这里不读配置。
pub(crate) fn stop_sessions(
    entries_to_stop: &[BotDirEntry],
    not_running_entries: Vec<BotDirEntry>,
    timing: StopTiming,
) -> StopOutcome {
    let mut outcome = StopOutcome {
        running_bot_count: entries_to_stop.len(),
        not_running_bot_entries: not_running_entries,
        ..Default::default()
    };

    if entries_to_stop.is_empty() {
        return outcome;
    }

    // 1. 会话名先统一算出来：拼不出会话名的目录（名字含非法字符）记一条失败明细，不参与后面几步。
    let mut target_sessions: Vec<(&str, String)> = Vec::with_capacity(entries_to_stop.len());
    for bot_dir_entry in entries_to_stop {
        let bot_name = bot_dir_entry.bot_name.as_str();
        match session_name_for(bot_name) {
            Ok(target_session_name) => target_sessions.push((bot_name, target_session_name)),
            Err(message) => outcome
                .failure_lines
                .push(format!("机器人 {bot_name}: {message}")),
        }
    }

    // 2. 一次性给所有会话发送 Ctrl+C，让 JVM 走 Shutdown Hook 退出。
    for (_, target_session_name) in &target_sessions {
        if let Err(message) = send_ctrl_c_to_session(target_session_name) {
            outcome.failure_lines.push(message);
        }
    }

    // 3. 留出退出时间。
    thread::sleep(Duration::from_millis(timing.force_after_ms));

    // 4. 仍未退出的会话强制终止。
    let still_alive_sessions = active_screen_sessions();
    for (bot_name, target_session_name) in &target_sessions {
        if still_alive_sessions.contains_key(target_session_name) {
            match force_quit_session(target_session_name) {
                Ok(_) => outcome
                    .force_stopped_bot_names
                    .push((*bot_name).to_string()),
                Err(message) => outcome.failure_lines.push(message),
            }
        }
    }

    // 5. 留出会话注销时间，再复核一次，确认账号已释放。
    thread::sleep(Duration::from_millis(timing.settle_ms));
    let final_sessions = active_screen_sessions();
    for (bot_name, target_session_name) in &target_sessions {
        if final_sessions.contains_key(target_session_name) {
            outcome
                .still_running_bot_names
                .push((*bot_name).to_string());
        } else {
            outcome.stopped_bot_names.push((*bot_name).to_string());
        }
    }

    outcome
}

/// 把停止明细整理成统一报告：基础信息 + 各类明细。
fn stop_report(outcome: StopOutcome) -> Report {
    let mut report = Report::untitled();

    report.set_counts(&[
        (
            "扫描机器人目录",
            outcome.running_bot_count + outcome.not_running_bot_entries.len(),
        ),
        ("成功停止", outcome.stopped_bot_names.len()),
        ("跳过 (原本未运行)", outcome.not_running_bot_entries.len()),
        (
            "停止失败",
            outcome.still_running_bot_names.len() + outcome.failure_lines.len(),
        ),
    ]);

    // 停止是当场复核过的（会话确实消失了），所以这里说得起"已停止"。
    report.succeeded = outcome
        .stopped_bot_names
        .iter()
        .map(|bot_name| {
            let note = if outcome.force_stopped_bot_names.contains(bot_name) {
                "（强制终止）"
            } else {
                ""
            };
            format!("机器人 {bot_name}: 已停止{note}")
        })
        .collect();

    report.skipped = outcome
        .not_running_bot_entries
        .iter()
        .map(|bot_dir_entry| format!("机器人 {}: 原本未运行，无需停止", bot_dir_entry.bot_name))
        .collect();

    for bot_name in &outcome.still_running_bot_names {
        report.failed.push(format!(
            "机器人 {bot_name}: 收尾后会话仍然存在，账号可能仍被占用"
        ));
    }
    report.failed.extend(outcome.failure_lines);

    report
}
