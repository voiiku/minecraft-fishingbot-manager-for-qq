//! 本进程（mc-qq-fishbot 与它里面的 kovi）自己的日志：终端与文件各写一份。
//!
//! * 位置：`<部署目录>/logs/年-月-日-时-分-秒.log` —— 与机器人日志同一套命名（`utils::create_timestamped_log_file`）；
//! * 内容：kovi、kovi-onebot 与本项目自己的 `log::*` 记录。本项目不再用 `println!` 打诊断信息，
//!   所以终端与文件是同一份内容（纯文本、不带颜色，文件干净）；
//! * 实现：自己实现 `log::Log`（kovi 自带的 logger 只写终端），直接依赖 `log` 与 `chrono`；
//! * 级别：`RUST_LOG` 优先（**只认单个级别词**），未设置时按配置的 `[kovi] debug`；
//! * 文件建不出来（部署目录不可写）**不致命**：退化成只写终端并留一条告警，不拖垮机器人。

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::Local;
use log::{LevelFilter, Log, Metadata, Record};

use crate::config;
use crate::utils;

/// 本进程日志所在的目录名：与可执行文件同级的 `logs/`。
const LOG_DIR_NAME: &str = "logs";

/// 日志文件首行（时间戳文件名之外再留一行，翻查时一眼看出这是哪一次启动）。
const LOG_FIRST_LINE: &str = "--- 启动 mc-qq-fishbot 进程 ---";

/// 每条记录的时间戳口径（本地时间，精确到秒；与文件名同一天）。
const LOG_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// 已安装的进程 logger：`log::set_logger` 要一个 `&'static`，所以放在静态里只建一次。
static PROCESS_LOGGER: OnceLock<ProcessLog> = OnceLock::new();

/// 初始化本进程日志：建日志文件 → 装 logger → 定级别 → 把结果自己写进日志。
///
/// 进程入口加载完配置就该调用它 —— 早于任何 `log::*`，也早于 `Bot::build`，
/// 之后 kovi 与库里的日志才会同时落到终端与文件。
pub fn init_process_log(debug: bool) {
    // 1. 先建日志文件。失败不当致命错误（见模块文档），原因攒起来稍后再写。
    let mut warnings: Vec<String> = Vec::new();
    let (log_path, log_file) = match open_process_log_file() {
        Ok((log_path, log_file)) => (Some(log_path), Some(log_file)),
        Err(message) => {
            warnings.push(message);
            (None, None)
        }
    };

    // 2. 装 logger 并定级别。
    let (level, level_warning) = resolve_log_level(debug);
    warnings.extend(level_warning);
    install_logger(log_file, level);

    // 3. 写结果：此刻 logger 已经装好，终端与文件都能看到这几行。
    for warning in warnings {
        log::warn!("{warning}");
    }
    if let Some(log_path) = log_path {
        log::info!("本进程日志: {}", log_path.display());
    }
}

/// 建本进程的日志文件：`<可执行文件同级>/logs/时间戳.log`。
///
/// 只做一件事（建文件与首行在 [`utils::create_timestamped_log_file`] 里，与机器人日志共用同一套命名），
/// 位置则来自 [`config::install_dir`] —— 部署目录。
fn open_process_log_file() -> Result<(PathBuf, File), String> {
    let process_logs_dir = config::install_dir()?.join(LOG_DIR_NAME);

    utils::create_timestamped_log_file(&process_logs_dir, LOG_FIRST_LINE)
}

/// 装 logger 并设定级别。
///
/// 重复调用（只会出现在出错时）不让它 panic：装不上就只写终端，并说明一次。
fn install_logger(log_file: Option<File>, level: LevelFilter) {
    let logger = PROCESS_LOGGER.get_or_init(|| ProcessLog {
        file: log_file.map(Mutex::new),
    });

    match log::set_logger(logger) {
        Ok(()) => log::set_max_level(level),
        Err(e) => println!("安装日志失败，本次只输出到终端: {e}"),
    }
}

/// 取日志级别：`RUST_LOG` 优先，未设置（或为空）时按配置的 `[kovi] debug` 决定。
///
/// 返回级别，以及「`RUST_LOG` 值认不出来」时的告警文案（调用方在 logger 装好后写进日志）。
fn resolve_log_level(debug: bool) -> (LevelFilter, Option<String>) {
    let fallback = if debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    let Some(raw) = std::env::var_os("RUST_LOG") else {
        return (fallback, None);
    };
    let raw = raw.to_string_lossy();
    let raw = raw.trim();
    if raw.is_empty() {
        return (fallback, None);
    }

    match raw.parse::<LevelFilter>() {
        Ok(level) => (level, None),
        Err(_) => (
            fallback,
            Some(format!(
                "RUST_LOG='{raw}' 不是单个级别词（off/error/warn/info/debug/trace），已按 {fallback} 处理"
            )),
        ),
    }
}

/// 进程日志：终端一份、文件一份（文件拿不到时就只有终端）。
struct ProcessLog {
    /// 日志文件句柄；`Mutex` 是因为 kovi 的任务与看门狗会并发写日志。
    file: Option<Mutex<File>>,
}

impl ProcessLog {
    /// 写出一行：先终端、再文件；任何一处失败都不影响另一处（日志不该拖垮进程）。
    fn write_line(&self, line: &str) {
        let mut stderr = std::io::stderr();
        let _ = stderr.write_all(line.as_bytes());
        let _ = stderr.flush();

        if let Some(file) = &self.file
            && let Ok(mut file) = file.lock()
        {
            // 写一行刷一行：进程被 kill -9 也不丢已经产生的日志。
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

impl Log for ProcessLog {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format!(
            "[{}] [{}] {}\n",
            Local::now().format(LOG_TIME_FORMAT),
            record.level(),
            record.args()
        );
        self.write_line(&line);
    }

    fn flush(&self) {
        if let Some(file) = &self.file
            && let Ok(mut file) = file.lock()
        {
            let _ = file.flush();
        }
    }
}
