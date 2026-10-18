//! 「谁在跑」的唯一口径：运行状态汇总、按运行状态分组、运行中的名字。
//!
//! 只查一次 `screen -list` 快照（[`active_screen_sessions`]），之后所有「在不在运行」的判断都基于
//! 同一份快照；状态判定与展示格式分别在 [`super::log_status`] 与 [`crate::show::status`]。
//!
//! **目录 × 会话的连接（join）只在这里做**：要「在运行的机器人」就调本模块的函数，
//! 其他模块不应重复遍历目录并匹配会话名，以免「在运行」的判定出现第二份实现。

use std::path::Path;

use crate::config;

use super::bot_dirs::{BotDirEntry, token_ready_bot_dir_entries};
use super::bot_log_files::{latest_log_path, read_log_lines_backwards_until};
use super::log_status::{ConnectionStatus, PauseStatus, StatusScanner};
use super::screen::{active_screen_sessions, is_bot_screen_running, session_name_for};
use super::uptime::process_elapsed_seconds;

/// 一个正在运行的机器人的状态快照。
#[derive(Debug, Clone)]
pub struct RunningBotInfo {
    /// 机器人名。
    pub bot_name: String,
    /// 最近一次日志给出的连接状态。
    pub connection_status: ConnectionStatus,
    /// 最近一次日志给出的暂停状态。
    pub pause_status: PauseStatus,
    /// 会话进程已经跑了多少秒。
    pub running_seconds: u64,
}

/// 按「在运行 / 未运行」把一批机器人目录分组（只查一次 `screen -list` 快照）。
///
/// 取哪批目录由调用方决定：停止与进程退出清理使用 [`super::bot_dirs::all_bot_dir_entries`]
/// （含 Token 缺失的孤儿），`/startall` 用 [`token_ready_bot_dir_entries`]。这里不按 Token 过滤。
pub fn split_bot_entries_by_running(
    bot_entries: &[BotDirEntry],
) -> (Vec<BotDirEntry>, Vec<BotDirEntry>) {
    let active_sessions = active_screen_sessions();

    let mut running_bot_entries = Vec::with_capacity(bot_entries.len());
    let mut not_running_bot_entries = Vec::new();
    for bot_dir_entry in bot_entries {
        // 拼不出会话名（名字含非法字符）就按"不在运行"算。
        let is_running = session_name_for(&bot_dir_entry.bot_name)
            .is_ok_and(|target_session_name| active_sessions.contains_key(&target_session_name));
        if is_running {
            running_bot_entries.push(bot_dir_entry.clone());
        } else {
            not_running_bot_entries.push(bot_dir_entry.clone());
        }
    }

    (running_bot_entries, not_running_bot_entries)
}

/// 该机器人正在运行时给出拒绝文案（供"运行中不许动"的操作统一使用）。
///
/// `requested_action` 是句尾短语，例如 `删除`、`更新 Token`。
/// 放在这里是因为它判断的就是"谁在跑"，而且几个指令的措辞只写一遍（`/bot del`、`/token` 都用它）。
pub fn refusal_if_running(bot_name: &str, requested_action: &str) -> Option<String> {
    is_bot_screen_running(bot_name).then(|| {
        format!("机器人 '{bot_name}' 正在运行，无法{requested_action}。请先执行 /stop {bot_name}。")
    })
}

/// 在给定目录里挑出正在运行的机器人名（`/commandall` 用）。
///
/// 就是 [`split_bot_entries_by_running`] 里「在运行」那一组的名字 —— 批量发指令只要名字，
/// 省下目录那一半。
pub fn running_bot_names(bot_entries: &[BotDirEntry]) -> Vec<String> {
    split_bot_entries_by_running(bot_entries)
        .0
        .into_iter()
        .map(|bot_dir_entry| bot_dir_entry.bot_name)
        .collect()
}

/// 扫描所有正在运行的机器人并返回其状态。
///
/// 只执行一次 `screen -list` 拿会话快照，并且只对在跑的实例读日志（每个实例只读一次）；
/// 结果按机器人名字典序（与 [`bot_dirs`] 的排序一致）。
pub fn running_bots_status() -> Result<Vec<RunningBotInfo>, String> {
    let active_sessions = active_screen_sessions();
    // 判定状态用的关键词整轮扫描都不变，取一次即可。
    let status_keywords = &config::get().keywords;
    let token_ready_bot_entries = token_ready_bot_dir_entries()?;

    let mut running_bot_infos = Vec::new();
    for bot_dir_entry in token_ready_bot_entries {
        // 只对确实在跑的会话解析状态，省掉无谓的日志读取；拼不出会话名的目录直接略过。
        let Some(session_pid) = session_name_for(&bot_dir_entry.bot_name)
            .ok()
            .and_then(|target_session_name| active_sessions.get(&target_session_name))
            .copied()
        else {
            continue;
        };

        let (connection_status, pause_status) =
            bot_statuses(&bot_dir_entry.bot_dir_path, status_keywords);

        running_bot_infos.push(RunningBotInfo {
            bot_name: bot_dir_entry.bot_name,
            connection_status,
            pause_status,
            running_seconds: process_elapsed_seconds(session_pid).unwrap_or(0),
        });
    }

    Ok(running_bot_infos)
}

/// 从一个机器人的最新日志里判定状态：**从末尾往回扫，判定已定就停**。
///
/// 三种"读不到"的情形区分得很清楚：没有日志文件、日志是空的、读不动 ——
/// 它们在 `/status` 里是三条不同的提示（排查方向完全不同）。
fn bot_statuses(
    bot_dir_path: &Path,
    status_keywords: &crate::config::KeywordsConfig,
) -> (ConnectionStatus, PauseStatus) {
    let Some(log_path) = latest_log_path(bot_dir_path) else {
        return (ConnectionStatus::NoLogYet, PauseStatus::Fishing);
    };

    let mut status_scanner = StatusScanner::new(status_keywords);
    let scanned_lines = match read_log_lines_backwards_until(&log_path, |line| {
        status_scanner.feed_line(line);
        status_scanner.is_decided()
    }) {
        Ok(scanned_lines) => scanned_lines,
        Err(_) => return (ConnectionStatus::UnreadableLog, PauseStatus::Fishing),
    };

    // 一行都没扫到 = 日志是空文件（与"没有日志文件"是不同的情况）。
    if scanned_lines.is_empty() {
        return (ConnectionStatus::EmptyLog, PauseStatus::Fishing);
    }

    status_scanner.finish()
}
