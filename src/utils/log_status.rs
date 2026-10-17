//! 从日志文本推断机器人状态（连接状态与暂停状态）。
//!
//! 这里全是纯函数：输入日志文本与配置里的关键词、输出状态，不读文件、不碰系统，
//! 所以能单独测试；日志文本由调用方准备（见 [`super::bot_log_files`] 与 [`super::running_bots`]）。
//!
//! 判定方向统一是"从日志末尾往前找最后一次出现的状态关键词"：日志是追加写的，
//! 最后一次出现才是当前状态。
//!
//! 关键词取自配置（`keywords.*`）：换游戏服或换 jar 版本后需要核对，否则这里会一直判不出状态。

use std::fmt::{self, Display, Formatter};

use crate::config::KeywordsConfig;

/// 机器人到游戏服务器的连接状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// 还没有日志文件（机器人没启动过）。
    NoLogYet,
    /// 日志文件存在，但读不出来。
    UnreadableLog,
    /// 日志文件是空的。
    EmptyLog,
    /// 日志里还没有出现任何能判断状态的关键词（刚启动）。
    Initializing,
    /// 已经进入游戏服务器。
    Online,
    /// 正在重连：进程还在，但此刻本人不在线。
    Reconnecting,
    /// 进程已结束或被手动中止。
    Stopped,
}

impl Display for ConnectionStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLogYet => write!(f, "未生成日志"),
            Self::UnreadableLog => write!(f, "读取日志失败"),
            Self::EmptyLog => write!(f, "日志为空"),
            Self::Initializing => write!(f, "就绪/初始中"),
            Self::Online => write!(f, "服务器在线"),
            Self::Reconnecting => write!(f, "重连/本人在线"),
            Self::Stopped => write!(f, "进程已结束"),
        }
    }
}

/// 机器人的暂停状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseStatus {
    /// 正在钓鱼。
    Fishing,
    /// 收到 `/pause` 之后暂停了钓鱼（但没下线）。
    Paused,
}

impl Display for PauseStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fishing => write!(f, "钓鱼中"),
            Self::Paused => write!(f, "已暂停"),
        }
    }
}

/// 逐行判定器：按「**从最新一行往旧**」的顺序喂行，两类状态各自记下第一个（也就是最新的）结论。
///
/// 做成逐行喂是为了能**提前停**：判定只关心"最后一次出现的关键词"，所以从末尾往回扫，
/// 两个状态都定了就不必再读更早的内容 —— 长跑账号的日志可能有几十 MB
/// （读取实现见 [`super::bot_log_files::read_log_lines_backwards_until`]，
/// 调用方见 [`super::running_bots`]）。
pub struct StatusScanner<'a> {
    keywords: &'a KeywordsConfig,
    connection: Option<ConnectionStatus>,
    pause: Option<PauseStatus>,
}

impl<'a> StatusScanner<'a> {
    /// 用配置里的关键词建一个判定器。
    pub fn new(keywords: &'a KeywordsConfig) -> Self {
        Self {
            keywords,
            connection: None,
            pause: None,
        }
    }

    /// 喂一行（调用方必须**从最新往旧**喂）；已经定下来的状态不会被更旧的行改写。
    pub fn feed_line(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }

        if self.connection.is_none() {
            self.connection = connection_status_of(trimmed, self.keywords);
        }
        if self.pause.is_none() {
            self.pause = pause_status_of(trimmed, self.keywords);
        }
    }

    /// 两类状态都有结论了吗 —— 有就可以停止读更早的日志了。
    pub fn is_decided(&self) -> bool {
        self.connection.is_some() && self.pause.is_some()
    }

    /// 收尾：没找到决定性关键词的状态按"刚启动 / 钓鱼中"处理（与整段判定的口径一致）。
    pub fn finish(self) -> (ConnectionStatus, PauseStatus) {
        (
            self.connection.unwrap_or(ConnectionStatus::Initializing),
            self.pause.unwrap_or(PauseStatus::Fishing),
        )
    }
}

/// 单行的连接状态：这一行是不是决定性的一行。
///
/// 重连提示要用前缀 + 后缀共同确认，避免把日志里凑巧出现的单个词当成状态。
fn connection_status_of(line: &str, keywords: &KeywordsConfig) -> Option<ConnectionStatus> {
    if line.contains(&keywords.stopped) {
        return Some(ConnectionStatus::Stopped);
    }
    if line.contains(&keywords.online) {
        return Some(ConnectionStatus::Online);
    }
    if line.contains(&keywords.reconnect_prefix) && line.contains(&keywords.reconnect_suffix) {
        return Some(ConnectionStatus::Reconnecting);
    }

    None
}

/// 单行的暂停状态：这一行是不是决定性的一行。
fn pause_status_of(line: &str, keywords: &KeywordsConfig) -> Option<PauseStatus> {
    if line.contains(&keywords.pause) {
        return Some(PauseStatus::Paused);
    }
    if line.contains(&keywords.unpause) {
        return Some(PauseStatus::Fishing);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用关键词：字段与真实配置一一对应，只是取值固定下来。
    fn keywords() -> KeywordsConfig {
        KeywordsConfig {
            online: "回到测试服".to_string(),
            reconnect_prefix: "钓鱼机器人将在".to_string(),
            reconnect_suffix: "秒内重新连接".to_string(),
            stopped: "机器人进程已结束".to_string(),
            pause: "已暂停钓鱼".to_string(),
            unpause: "已恢复钓鱼".to_string(),
            filter_regex: "XP".to_string(),
        }
    }

    /// 按生产路径判定：从最新一行往旧喂 [`StatusScanner`]，判定已定就停。
    ///
    /// 注意这里不产出 `EmptyLog` —— 那是"一行都没扫到"时**调用方**的结论
    /// （见 `running_bots::bot_statuses`），判定器对空输入只会得到"没找到关键词"。
    fn statuses_of(log_text: &str, keywords: &KeywordsConfig) -> (ConnectionStatus, PauseStatus) {
        let mut scanner = StatusScanner::new(keywords);
        for line in log_text.lines().rev() {
            scanner.feed_line(line);
            if scanner.is_decided() {
                break;
            }
        }

        scanner.finish()
    }

    /// 取最后一次出现的关键词：结束过又重新上线，最终状态应是在线。
    #[test]
    fn latest_keyword_wins() {
        let log_text = "回到测试服\n机器人进程已结束\n回到测试服\n";
        let (connection_status, _) = statuses_of(log_text, &keywords());

        assert_eq!(connection_status, ConnectionStatus::Online);
    }

    /// 逐行判定器记的是**第一个**（=最新的）结论：更旧的行不会把它改回去 ——
    /// "从末尾往回读、判定已定就停"正是靠这条性质才成立。
    #[test]
    fn scanner_keeps_the_newest_conclusion() {
        let keywords = keywords();
        let mut scanner = StatusScanner::new(&keywords);
        scanner.feed_line(&keywords.stopped); // 较新：进程已结束
        scanner.feed_line(&keywords.online); // 更旧：曾经在线

        assert_eq!(scanner.finish().0, ConnectionStatus::Stopped);
    }

    /// 关键词一个都没出现时是"就绪/初始中"，不是"掉线"。
    #[test]
    fn no_keyword_means_initializing() {
        let (connection_status, pause_status) = statuses_of("正在加载配置\n", &keywords());

        assert_eq!(connection_status, ConnectionStatus::Initializing);
        assert_eq!(pause_status, PauseStatus::Fishing);
    }

    /// 空输入不会被判成"有结论"：`EmptyLog` 是调用方看"一行都没扫到"才给出的分类
    /// （见 `running_bots::bot_statuses`），这里只确认判定器本身不瞎猜。
    #[test]
    fn blank_input_is_never_decided() {
        let keywords = keywords();
        let mut scanner = StatusScanner::new(&keywords);
        scanner.feed_line("");

        assert!(!scanner.is_decided());
        assert_eq!(
            scanner.finish(),
            (ConnectionStatus::Initializing, PauseStatus::Fishing)
        );
    }

    /// 暂停与恢复按"最后一次出现的"判定。
    #[test]
    fn pause_status_follows_the_last_line() {
        let keywords = keywords();
        let paused = format!("{}\n", keywords.pause);
        assert_eq!(statuses_of(&paused, &keywords).1, PauseStatus::Paused);

        let resumed = format!("{}\n{}\n", keywords.pause, keywords.unpause);
        assert_eq!(statuses_of(&resumed, &keywords).1, PauseStatus::Fishing);
    }

    /// 重连提示要求前缀与后缀同时出现，缺一个不算（否则日志里的半个词会误判）。
    #[test]
    fn reconnect_needs_both_affixes() {
        let keywords = keywords();

        let complete = format!(
            "{}{}\n",
            keywords.reconnect_prefix, keywords.reconnect_suffix
        );
        assert_eq!(
            statuses_of(&complete, &keywords).0,
            ConnectionStatus::Reconnecting
        );

        let partial = format!("{}\n", keywords.reconnect_prefix);
        assert_eq!(
            statuses_of(&partial, &keywords).0,
            ConnectionStatus::Initializing
        );
    }
}
