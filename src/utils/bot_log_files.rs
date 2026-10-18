//! 机器人日志文件：创建、挑选、读取，以及 FishingBot 自己那份日志的处置。
//!
//! `<机器人目录>/<runtime.logs_dir>` 仅包含两类文件：
//!
//! * `时间戳.log` —— **本程序日志**：[`create_bot_log_file`] 在启动会话前创建，
//!   会话用 `grep -v -E` 过滤 JVM 输出后 `tee -a` 追加；`/log` 与 `/status` 读它；
//! * `raw/log0.log` —— **FishingBot 的日志**：指向 `/dev/null` 的符号链接，内容由内核丢弃。
//!   FishingBot 建不出这个文件会直接拒绝启动，所以给它一个"能写、但什么都不存"的目标。
//!
//! 只认时间戳命名（[`is_own_log_file_name`]）：早先按"最后修改时间"挑 `.log` 时，
//! 实时写入的 `log0.log` 修改时间更晚，`/log` 因此读取到未过滤的高频日志行。
//!
//! 本进程自己的日志（`<部署目录>/logs/`）用同一套命名，但属于另一条线，见 `src/process_log.rs`。

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::Local;

use crate::config;

/// 日志文件名里的时间戳口径：`2026-09-28-01-47-01`（本地时间，精确到秒）。
const LOG_NAME_TIME_FORMAT: &str = "%Y-%m-%d-%H-%M-%S";

/// [`LOG_NAME_TIME_FORMAT`] 的长度。
const LOG_NAME_TIME_LENGTH: usize = 19;

/// 本程序日志文件的固定首行。
const BOT_LOG_FIRST_LINE: &str = "--- 启动机器人 ---";

/// FishingBot 存放日志的子目录名：`<机器人日志目录>/raw`。
const GAME_LOG_DIR_NAME: &str = "raw";

/// FishingBot 的日志文件名：它的 `FileHandler` 模式是 `log%g.log`，第一次运行是 `log0.log`。
const GAME_LOG_FILE_NAME: &str = "log0.log";

/// 丢弃设备：FishingBot 的日志内容写到这里，内核直接丢掉。
const DISCARD_DEVICE: &str = "/dev/null";

/// 机器人日志目录：`<机器人目录>/<runtime.logs_dir>`。
///
/// 纯函数：目录名由调用方给（[`create_bot_log_file`] 与 [`latest_log_path`] 各自从配置里读
/// `runtime.logs_dir` 这一个字段），因此日志目录层级可单独测试。
pub fn bot_logs_dir(bot_dir: &Path, logs_dir_name: &str) -> PathBuf {
    bot_dir.join(logs_dir_name)
}

/// 清空 FishingBot 的日志目录，把它的日志文件指向 [`DISCARD_DEVICE`]，返回这个目录。
///
/// 参数是**机器人的日志目录**（[`bot_logs_dir`] 的结果），不是机器人目录 —— 名字里带
/// `fishingbot` 用于与本程序日志区分（模块文档中的两条日志线）。
/// 这是启动流程的第二步（见 [`super::startup`]）：`-logsdir` 用它，[`create_bot_log_file`] 进它。
/// 清不掉、建不出只写一条告警、不打断启动 —— 最坏情况是 FishingBot 建不出文件而拒绝启动，
/// 它会在自己的输出里说明。
pub fn reset_fishingbot_logs_dir(bot_logs_dir: &Path) -> PathBuf {
    let fishingbot_logs_dir = bot_logs_dir.join(GAME_LOG_DIR_NAME);

    // 目录不存在时 `remove_dir_all` 会报错，先判断存在性。
    if fishingbot_logs_dir.exists()
        && let Err(e) = fs::remove_dir_all(&fishingbot_logs_dir)
    {
        log::warn!(
            "清空 FishingBot 日志目录 {} 失败: {e}",
            fishingbot_logs_dir.display()
        );
    }
    if let Err(e) = fs::create_dir_all(&fishingbot_logs_dir) {
        log::warn!(
            "创建 FishingBot 日志目录 {} 失败: {e}（它可能因此拒绝启动）",
            fishingbot_logs_dir.display()
        );
    }

    let game_log_file = fishingbot_logs_dir.join(GAME_LOG_FILE_NAME);
    if let Err(e) = discard_game_log_file(&game_log_file) {
        log::warn!(
            "把 FishingBot 的日志文件 {} 指向 {DISCARD_DEVICE} 失败: {e}（会多出一份未过滤的日志）",
            game_log_file.display()
        );
    }

    fishingbot_logs_dir
}

/// 把 FishingBot 的日志文件做成 [`DISCARD_DEVICE`] 的符号链接。
#[cfg(unix)]
fn discard_game_log_file(game_log_file: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(DISCARD_DEVICE, game_log_file)
}

/// 非 Unix 平台（本项目不部署到这些平台，仅为运行测试）创建空文件占位。
#[cfg(not(unix))]
fn discard_game_log_file(game_log_file: &Path) -> std::io::Result<()> {
    fs::File::create(game_log_file).map(|_| ())
}

/// 判断一个文件名是不是本项目写的日志（形如 `2026-09-28-01-47-01.log`）。
///
/// 只认这种时间戳命名：长度固定、数字与连字符的位置也固定，所以 FishingBot 自己写的
/// `log0.log`、`log1.log` 等绝不可能被误判成本程序的日志（原因见模块文档）。
fn is_own_log_file_name(file_name: &str) -> bool {
    let Some(time_part) = file_name.strip_suffix(".log") else {
        return false;
    };
    if time_part.len() != LOG_NAME_TIME_LENGTH {
        return false;
    }

    time_part.char_indices().all(|(index, ch)| {
        if matches!(index, 4 | 7 | 10 | 13 | 16) {
            ch == '-'
        } else {
            ch.is_ascii_digit()
        }
    })
}

/// 在 `dir` 下新建一份时间戳命名的日志文件，写入首行 `first_line`，返回路径与可写句柄。
///
/// 机器人日志（[`create_bot_log_file`]）与本进程日志（`src/process_log.rs`）共用它，
/// 所以「日志文件叫什么」这件事只有这一处实现 —— 名字里带 `timestamped` 就是说这件事：
/// 文件名由**本地时间**生成（`年-月-日-时-分-秒.log`），调用方给不出其他命名。
///
/// 句柄的写入位置在首行之后，拿到的调用方可以接着往后写（本进程日志就是这么用的）；
/// 机器人日志用不到句柄，丢掉即可 —— 之后由会话里的 `tee -a` 往里追加。
///
/// 目录不存在会一并创建（`create_dir_all`），因此调用方无需预先创建目录。
/// 同一秒内重复创建会复用同一文件名（`create` 截断旧内容），不会产生多余空文件。
pub fn create_timestamped_log_file(
    dir: &Path,
    first_line: &str,
) -> Result<(PathBuf, File), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建日志目录 {} 失败：{e}", dir.display()))?;

    let log_path = dir.join(format!("{}.log", Local::now().format(LOG_NAME_TIME_FORMAT)));

    let mut file = File::create(&log_path)
        .map_err(|e| format!("创建日志文件 {} 失败：{e}", log_path.display()))?;
    writeln!(file, "{first_line}")
        .map_err(|e| format!("写入日志首行 {} 失败：{e}", log_path.display()))?;

    Ok((log_path, file))
}

/// 为一次机器人运行创建日志文件：`<机器人目录>/<runtime.logs_dir>/时间戳.log`。
///
/// 这是启动流程的第一步（见 [`super::startup::launch_bot_session`]）：日志文件在启动会话
/// **之前**创建，会话只负责追加；创建时机与文件名可直接在代码中确认，
/// 不藏在 shell 脚本的 `mkdir` / `date` 里。
pub fn create_bot_log_file(bot_dir: &Path) -> Result<PathBuf, String> {
    let logs_dir_name = &config::get().runtime.logs_dir;

    create_timestamped_log_file(&bot_logs_dir(bot_dir, logs_dir_name), BOT_LOG_FIRST_LINE)
        .map(|(log_path, _file)| log_path)
}

/// 机器人最新的一份日志文件（`logs/` 下最后修改的那份**过滤后的**日志）。
///
/// 每个会话启动时写一个新文件，所以"最新"的那份就是当前或最近一次运行的日志；
/// 名字不符合本项目命名规则的文件（FishingBot 自己写的原始日志）一律不参与挑选。
pub fn latest_log_path(bot_dir: &Path) -> Option<PathBuf> {
    let logs_dir = bot_logs_dir(bot_dir, &config::get().runtime.logs_dir);

    fs::read_dir(&logs_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|file_name| file_name.to_str())
                    .is_some_and(is_own_log_file_name)
        })
        .max_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok())
}

/// 每次从文件末尾往回读的字节数：够小以省内存，够大以减少系统调用次数。
const BACKWARD_READ_BLOCK_BYTES: usize = 16 * 1024;

/// 从日志末尾往回逐行读，直到 `is_enough` 返回 `true`；返回扫到的行（**文件顺序：旧→新**）。
///
/// 为什么不是"直接读全文"：状态判定只关心**最后一次**出现的关键词
/// （见 [`super::log_status::StatusScanner`]），所以从最新一行往回扫、判定已定就能停。
/// 长期运行账号的日志可能达到几十 MB，整份读取缓慢且占用内存；此处任何情况下都不会更差
/// （最坏情况就是读完全文），内存只与"往回看了多远"成正比。
///
/// 行在拼成完整 UTF-8 文本后才切分（块之间保留半行），因此不会因块边界截断字符。
/// 文件为空时返回空 `Vec` —— 调用方据此区分"还没有日志"（文件不存在）与"日志是空的"。
pub fn read_log_lines_backwards_until(
    log_path: &Path,
    mut is_enough: impl FnMut(&str) -> bool,
) -> Result<Vec<String>, String> {
    let mut file = File::open(log_path).map_err(|e| format!("无法打开日志文件：{e}"))?;
    let mut position = file
        .metadata()
        .map_err(|e| format!("读取日志文件信息失败：{e}"))?
        .len();

    // 已扫到的行（新→旧），最后反转成文件顺序返回。
    let mut scanned_lines: Vec<String> = Vec::new();
    // 上一块开头那半行：它的前半部分在更旧的一块里，拼起来才是完整的一行。
    let mut pending_prefix: Vec<u8> = Vec::new();

    while position > 0 {
        let read_size = std::cmp::min(BACKWARD_READ_BLOCK_BYTES as u64, position) as usize;
        position -= read_size as u64;

        file.seek(SeekFrom::Start(position))
            .map_err(|e| format!("定位日志文件失败：{e}"))?;
        let mut chunk = vec![0u8; read_size];
        file.read_exact(&mut chunk)
            .map_err(|e| format!("读取日志内容失败：{e}"))?;

        // 先拼字节再解码：跨块的多字节字符因此不会被解码成乱码。
        chunk.extend_from_slice(&pending_prefix);
        let text = String::from_utf8_lossy(&chunk).into_owned();
        let mut lines: Vec<&str> = text.split('\n').collect();

        // 不是文件开头时，第一段是"被切开的半行"，留给更旧的一块补全。
        pending_prefix = if position > 0 {
            lines.remove(0).as_bytes().to_vec()
        } else {
            Vec::new()
        };

        let mut is_enough_reached = false;
        for line in lines.iter().rev() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if is_enough(line) {
                is_enough_reached = true;
            }
            scanned_lines.push(line.to_string());
            if is_enough_reached {
                break;
            }
        }

        if is_enough_reached {
            break;
        }
    }

    scanned_lines.reverse();
    Ok(scanned_lines)
}

/// 读取日志文件的最后 `max_lines` 行。
///
/// 使用滑动窗口仅保留最后 N 行：日志再大也不会整份读入内存。
pub fn read_last_lines(log_path: &Path, max_lines: usize) -> Result<Vec<String>, String> {
    // 行数为 0 时不该退化成"读完整个文件"，这里兜底成"只留最后一行"。
    let max_lines = max_lines.max(1);

    let file = File::open(log_path).map_err(|e| format!("无法打开日志文件：{e}"))?;
    let log_reader = BufReader::new(file);

    let mut ring_buffer: VecDeque<String> = VecDeque::with_capacity(max_lines);
    for line_result in log_reader.lines() {
        let line = line_result.map_err(|e| format!("读取日志内容失败：{e}"))?;
        // 窗口满了先弹掉最老的一行，保证任何时刻都只留最后 N 行。
        if ring_buffer.len() == max_lines {
            ring_buffer.pop_front();
        }
        ring_buffer.push_back(line);
    }

    Ok(ring_buffer.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 日志目录就在机器人目录下的 `logs_dir` 那一层（名字由配置给，这里直接传参对照）。
    #[test]
    fn bot_logs_dir_uses_the_configured_name() {
        let bot_dir = Path::new("/fishingbots/bots/123456/duck");

        assert_eq!(
            bot_logs_dir(bot_dir, "logs"),
            Path::new("/fishingbots/bots/123456/duck/logs")
        );
        assert_eq!(
            bot_logs_dir(bot_dir, "log"),
            Path::new("/fishingbots/bots/123456/duck/log")
        );
    }

    /// 本项目写的日志（[`create_timestamped_log_file`] 按 `LOG_NAME_TIME_FORMAT` 命名）要能认出来。
    #[test]
    fn own_log_names_are_recognized() {
        assert!(is_own_log_file_name("2026-09-28-01-47-01.log"));
    }

    /// FishingBot 自己写的原始日志（Java FileHandler 的 `log0.log` 等）不能识别为本程序日志：
    /// 识别错误时 `/log` 会读取未过滤内容（`+24.50XP` 这类高频日志行）。
    #[test]
    fn game_log_names_are_rejected() {
        let names = [
            "log0.log",
            "log1.log",
            "log.log",
            "logs.log",
            "FishingBot.log",
        ];

        for name in names {
            assert!(!is_own_log_file_name(name), "{name} 不该被当成我们的日志");
        }
    }

    /// 长得像时间戳但格式不对（长度、分隔符位置、后缀有出入）的也要拒绝。
    #[test]
    fn malformed_timestamp_names_are_rejected() {
        let names = [
            "2026-09-28-01-47-01.log.bak",
            "2026-09-28-01-47-01.logs",
            "2026_09_28-01-47-01.log",
            "2026-09-28-01-47-1.log",
            "20260928-01-47-01.log",
            "2026-09-28-01-47-.log",
        ];

        for name in names {
            assert!(!is_own_log_file_name(name), "{name} 不该被当成我们的日志");
        }
    }

    /// 建出来的文件必须能被 [`is_own_log_file_name`] 认出来，而且首行与句柄都能用。
    ///
    /// 命名这件事现在由 Rust 生成、又被 `/log` 的挑选口径校验，两边任一处改了写法，
    /// `/log` 会错误地报告"还没有日志"；该测试用于保持两处命名规则一致。
    #[test]
    fn created_log_file_matches_own_name_rule() {
        let dir = temp_test_dir("create-log-file");
        let first_line = "--- 测试首行 ---";

        let (log_path, mut file) =
            create_timestamped_log_file(&dir, first_line).expect("应当能建出日志文件");

        let file_name = log_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("日志文件名应当能转成字符串");
        assert!(is_own_log_file_name(file_name), "{file_name} 应当被认出来");
        assert_eq!(
            fs::read_to_string(&log_path).expect("读回日志"),
            format!("{first_line}\n")
        );

        // 句柄的写入位置就在首行之后：接着写就是追加，不会截掉首行。
        writeln!(file, "测试追加").expect("写日志");
        assert_eq!(
            fs::read_to_string(&log_path).expect("读回日志"),
            format!("{first_line}\n测试追加\n")
        );

        fs::remove_dir_all(&dir).expect("清理临时目录");
    }

    /// 每个测试使用独立临时目录（带进程号，避免并发测试相互干扰）。
    fn temp_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mc-qq-fishbot-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        dir
    }

    /// FishingBot 的日志目录与文件都要准备好：它建不出文件会直接拒绝启动。
    #[test]
    fn game_logs_dir_is_prepared() {
        let bot_logs_dir = temp_test_dir("prepare-game-logs");

        let game_logs_dir = reset_fishingbot_logs_dir(&bot_logs_dir);

        assert_eq!(game_logs_dir, bot_logs_dir.join("raw"));
        assert!(game_logs_dir.is_dir());
        assert!(game_logs_dir.join("log0.log").exists(), "日志文件要能写");

        fs::remove_dir_all(&bot_logs_dir).expect("清理临时目录");
    }

    /// 写进 FishingBot 日志文件的内容必须被丢掉（它是 `/dev/null` 的符号链接）：
    /// 磁盘上只能有一份日志，即本程序日志。
    #[cfg(unix)]
    #[test]
    fn game_log_file_discards_writes() {
        let bot_logs_dir = temp_test_dir("discard-game-log");

        let game_log_file = reset_fishingbot_logs_dir(&bot_logs_dir).join("log0.log");
        assert_eq!(
            fs::read_link(&game_log_file).expect("应当是符号链接"),
            Path::new("/dev/null")
        );

        fs::write(&game_log_file, "未过滤的刷屏行\n").expect("写 FishingBot 的日志");
        assert_eq!(fs::metadata(&game_log_file).expect("看文件大小").len(), 0);

        fs::remove_dir_all(&bot_logs_dir).expect("清理临时目录");
    }

    /// 不保留上次运行的文件：`raw/` 每次启动前清空重建。
    #[test]
    fn resetting_game_logs_dir_clears_previous_run() {
        let bot_logs_dir = temp_test_dir("reset-game-logs");

        let game_logs_dir = reset_fishingbot_logs_dir(&bot_logs_dir);
        fs::write(game_logs_dir.join("log1.log"), "上一轮留下的\n").expect("造一个上一轮的文件");

        let game_logs_dir_again = reset_fishingbot_logs_dir(&bot_logs_dir);

        assert_eq!(game_logs_dir_again, game_logs_dir);
        assert!(
            !game_logs_dir.join("log1.log").exists(),
            "上一轮的文件不该被保留"
        );
        assert!(game_logs_dir.join("log0.log").exists());

        fs::remove_dir_all(&bot_logs_dir).expect("清理临时目录");
    }
}
