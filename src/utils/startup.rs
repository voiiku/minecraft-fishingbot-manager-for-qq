//! 启动与重启：`/start`、`/startall`、`/restart`、`/restartall` 共用同一套流程。
//!
//! 启动是**四个显式步骤**（见 [`start_bot_sessions`]），shell 脚本只负责"跑 JVM → 过滤 → 追加日志"：
//!
//! 1. 建这次运行的日志文件（[`super::bot_log_files::create_bot_log_file`]）；
//! 2. 准备 FishingBot 自己的日志目录（[`super::bot_log_files::reset_fishingbot_logs_dir`]：内容丢弃）；
//! 3. 拉起 screen 会话（[`launch_bot_session`]）；
//! 4. 错峰 + 一次 `screen -list` 快照复核会话是否出现。
//!
//! 重启 = 停止（[`super::shutdown::stop_sessions`]）+ 上面这套启动流程。

use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::config;

use super::bot_dirs::BotDirEntry;
use super::bot_log_files::{bot_logs_dir, create_bot_log_file, reset_fishingbot_logs_dir};
use super::report::Report;
use super::running_bots::split_bot_entries_by_running;
use super::screen::{active_screen_sessions, session_name_for};
use super::shutdown::{StopTiming, stop_sessions};

/// 拉起一批会话时要等多久：单个指令不错峰，批量指令要错峰。
///
/// 把「两个毫秒数」收成一个具名参数，调用处就不会再出现 `start_bot_sessions(&entries, 0, 1500)`
/// 这种要回头查文档才知道 `0` 是什么的写法。
#[derive(Debug, Clone, Copy)]
pub struct LaunchTiming {
    /// 每拉起一个机器人之后等多久，再拉起下一个（单个指令传 0，不需要错峰）。
    pub stagger_ms: u64,
    /// 全部拉起之后再等多久才复核会话（给 JVM 一点初始化时间，让内存不足先暴露出来）。
    pub settle_ms: u64,
}

impl LaunchTiming {
    /// 单个机器人指令：只有一个会话，不需要错峰。
    pub fn single() -> Self {
        let timing = &config::get().timing;
        Self {
            stagger_ms: 0,
            settle_ms: timing.single_settle_ms,
        }
    }

    /// 批量指令：错峰拉起多个 JVM（避免内存峰值），等待时间也更长。
    pub fn batch() -> Self {
        let timing = &config::get().timing;
        Self {
            stagger_ms: timing.batch_stagger_ms,
            settle_ms: timing.batch_settle_ms,
        }
    }
}

/// 错峰拉起一批机器人，等待 `timing.settle_ms` 后统一复核会话是否出现。
///
/// 前置条件是 **Token 已就绪**（条目里的 `bot_token_status`）：缺 Token 的账号登录必然失败，所以不拉起、
/// 只记一条失败明细。这一条对所有启动路径生效（`/start` 在业务层还会先拦一道，好给单机指令一句
/// 清楚的文案），`/restart`、`/restartall` 的批次就靠它兜底。
///
/// 成功明细只说「启动命令已发送，会话已出现」：会话存在不等于已上线（登录、进服由 JVM 自己跑）。
pub fn start_bot_sessions(bots_to_launch: &[BotDirEntry], timing: LaunchTiming) -> Report {
    let mut report = Report::untitled();

    if bots_to_launch.is_empty() {
        return report;
    }

    let mut failure_lines: Vec<String> = Vec::new();
    // 这个函数只用到一项配置：日志目录名（拼给 FishingBot 的 `-logsdir` 与建日志文件都用它）。
    let logs_dir_name = &config::get().runtime.logs_dir;
    // 没被拉起过的机器人（前置检查或第一步就失败）：复核阶段要跳开，否则会多出一条误导信息。
    let mut not_launched_bot_names: Vec<String> = Vec::new();
    for (index, bot_dir_entry) in bots_to_launch.iter().enumerate() {
        let bot_name = &bot_dir_entry.bot_name;
        let bot_dir_path = &bot_dir_entry.bot_dir_path;

        // 第零步：没有 Token 就没有可用的登录凭据，拉起来只会白白占内存与游戏账号。
        if !bot_dir_entry.bot_token_status.is_ready() {
            log::warn!("机器人 {bot_name} 缺少 Token 文件，未拉起");
            not_launched_bot_names.push(bot_name.clone());
            failure_lines.push(format!("机器人 {bot_name}: 缺少 Token 文件，未拉起"));
            continue;
        }

        // 第一步：建这次运行的日志文件；建不出来就不拉会话（没有日志就无从查起）。
        // 失败原因（带路径）写进本进程日志，回复里只给结论。
        let log_path = match create_bot_log_file(bot_dir_path) {
            Ok(log_path) => log_path,
            Err(message) => {
                log::error!("机器人 {bot_name} 的日志文件创建失败: {message}");
                not_launched_bot_names.push(bot_name.clone());
                failure_lines.push(format!(
                    "机器人 {bot_name}: 无法创建日志文件（检查磁盘空间与目录权限）"
                ));
                continue;
            }
        };

        // 第二步：让 FishingBot 自己的日志写向 /dev/null（磁盘上只留我们的那份日志）。
        let fishingbot_logs_dir =
            reset_fishingbot_logs_dir(&bot_logs_dir(bot_dir_path, logs_dir_name));

        // 第三步：拉起会话，JVM 的输出过滤后追加进第一步建好的日志文件。
        // 失败（含机器人名拼不出会话名）同样进"未拉起"名单，复核阶段就跳过它，不会多报一条内存不足。
        if let Err(message) =
            launch_bot_session(bot_name, bot_dir_path, &log_path, &fishingbot_logs_dir)
        {
            not_launched_bot_names.push(bot_name.clone());
            failure_lines.push(message);
        }

        // 第四步：错峰，避免多个 JVM 同时启动把内存打满（复核在全部拉起之后统一做）。
        // 最后一台不必等：后面还有 settle 的等待，再睡一次只是白拖时间。
        let is_last_bot = index + 1 == bots_to_launch.len();
        if timing.stagger_ms > 0 && !is_last_bot {
            thread::sleep(Duration::from_millis(timing.stagger_ms));
        }
    }

    // 等 JVM 初始化，让内存不足一类的问题先暴露出来再复核。
    if timing.settle_ms > 0 {
        thread::sleep(Duration::from_millis(timing.settle_ms));
    }

    let active_sessions = active_screen_sessions();
    let mut started_lines: Vec<String> = Vec::with_capacity(bots_to_launch.len());
    for bot_dir_entry in bots_to_launch {
        let bot_name = &bot_dir_entry.bot_name;
        if not_launched_bot_names.contains(bot_name) {
            continue;
        }

        let session_appeared = session_name_for(bot_name)
            .is_ok_and(|target_session_name| active_sessions.contains_key(&target_session_name));
        if session_appeared {
            started_lines.push(format!("机器人 {bot_name}: 启动命令已发送，会话已出现"));
        } else {
            failure_lines.push(format!(
                "机器人 {bot_name}: 启动指令已发送，但进程不存在（通常是内存不足）"
            ));
        }
    }

    report.succeeded = started_lines;
    report.failed = failure_lines;

    report
}

/// 重启一批机器人：先停掉目标中正在运行的，再把可以启动的全部拉起。
///
/// 缺 Token 的目标**在停止之前**就被剔掉（只写一条失败明细）：重启是"先停再起"，而它起不来，
/// 白停一次会把一个可能还在钓鱼的账号弄下线。
/// 对原本未运行的目标等价于直接启动；停止阶段未能注销干净的会跳过重启，
/// 避免两个 JVM 争用同一账号。
pub fn restart_bot_sessions(bots_to_restart: &[BotDirEntry], timing: LaunchTiming) -> Report {
    // 缺 Token 的先剔掉 —— 必须在停止之前算出来（见函数文档）。
    let (bot_entries_without_token, restartable_bot_entries): (Vec<BotDirEntry>, Vec<BotDirEntry>) =
        bots_to_restart
            .iter()
            .cloned()
            .partition(|bot_dir_entry| !bot_dir_entry.bot_token_status.is_ready());

    // 只执行一次 `screen -list`，把目标分成「原本在运行 / 原本未运行」。
    let (running_bot_entries, not_running_bot_entries) =
        split_bot_entries_by_running(&restartable_bot_entries);

    let stop_outcome = stop_sessions(
        &running_bot_entries,
        not_running_bot_entries,
        StopTiming::from_config(),
    );

    // 停止后仍在运行的跳过重启，其余（停成功的与原本未运行的）统一启动。
    let bots_to_start: Vec<BotDirEntry> = restartable_bot_entries
        .iter()
        .filter(|bot_dir_entry| {
            !stop_outcome
                .still_running_bot_names
                .contains(&bot_dir_entry.bot_name)
        })
        .cloned()
        .collect();

    let mut report = Report::untitled();
    for bot_dir_entry in &bot_entries_without_token {
        let bot_name = &bot_dir_entry.bot_name;
        report.failed.push(format!(
            "机器人 {bot_name}: 缺少 Token 文件，重启会登录失败，已保持原状（先 /token 再重启）"
        ));
    }
    for bot_name in &stop_outcome.still_running_bot_names {
        report.failed.push(format!(
            "机器人 {bot_name}: 停止阶段未能注销会话，已跳过重启"
        ));
    }
    report.failed.extend(stop_outcome.failure_lines);
    report.absorb(start_bot_sessions(&bots_to_start, timing));

    report
}

/// 拉起单个机器人的会话：只做一件事，校验与准备都由 [`start_bot_sessions`] 的前两步完成。
fn launch_bot_session(
    bot_name: &str,
    bot_dir_path: &Path,
    log_path: &Path,
    fishingbot_logs_dir: &Path,
) -> Result<(), String> {
    let session_name = session_name_for(bot_name)?;
    let launch_command = build_launch_command(log_path, fishingbot_logs_dir);

    let mut cmd = Command::new("screen");
    // 会话的工作目录就是机器人目录（FishingBot 以它为基准找相对路径）。
    cmd.current_dir(bot_dir_path);
    // -dmS：后台分离模式创建会话并执行命令。
    cmd.args(["-dmS", &session_name, "bash", "-c", &launch_command]);
    cmd.status()
        .map_err(|e| format!("启动机器人 {bot_name} 失败: {e}"))?;

    log::info!(
        "尝试拉起机器人 {bot_name} 的 Screen 会话 ({session_name})，日志: {}",
        log_path.display()
    );
    Ok(())
}

/// 拉起会话时要拼进命令的配置取值：一次给全，字段都有名字，不会传错位置。
///
/// 这几个字段散在配置的三个表里（`runtime` / `paths` / `keywords`），
/// 收成一个参数结构体是为了让 [`launch_command_line`] 只依赖"值"、不依赖"配置放在哪"。
#[derive(Debug, Clone, Copy)]
pub struct LaunchCommandConfig<'a> {
    /// `runtime.java_opts`：JVM 参数，未加引号内联进命令。
    pub java_opts: &'a str,
    /// `runtime.token_file`：Token 文件名（相对机器人目录）。
    pub token_file_name: &'a str,
    /// `keywords.filter_regex`：丢掉刷屏行的正则（传给 `grep -v -E`）。
    pub filter_regex: &'a str,
    /// `paths.jar`：FishingBot 的 jar。
    pub jar: &'a Path,
    /// `paths.game_config`：FishingBot 的共享配置。
    pub game_config: &'a Path,
}

/// 拼出会话里执行的启动脚本：跑 JVM → 过滤 → 追加进日志文件。
///
/// 纯函数（取值全部由调用方给），所以这条**最危险**的命令能对着固定取值逐字断言 ——
/// 见本文件测试；真正读配置的是 [`build_launch_command`]。
///
/// * `-logsdir` 指向 FishingBot 的日志目录（里面是指向 `/dev/null` 的符号链接，内容丢弃）；
/// * `grep --line-buffered -v -E` 丢掉刷屏行（**命中即丢**），`--line-buffered` 让日志实时落盘，
///   而不是等管道缓冲满。
pub fn launch_command_line(
    log_path: &Path,
    fishingbot_logs_dir: &Path,
    config: LaunchCommandConfig<'_>,
) -> String {
    // java_opts 为空时不补空格，避免参数与 -jar 粘连。
    let jvm_opts = if config.java_opts.trim().is_empty() {
        String::new()
    } else {
        format!("{} ", config.java_opts.trim())
    };

    format!(
        concat!(
            "java {jvm_opts}-jar '{jar}' -nogui -config '{game_config}' ",
            "-refreshToken '{token_file}' -logsdir '{logsdir}' 2>&1 ",
            "| grep --line-buffered -v -E '{filter_regex}' | tee -a '{log_path}'"
        ),
        jvm_opts = jvm_opts,
        jar = config.jar.display(),
        game_config = config.game_config.display(),
        token_file = config.token_file_name,
        logsdir = fishingbot_logs_dir.display(),
        filter_regex = config.filter_regex,
        log_path = log_path.display()
    )
}

/// 启动流程的取值层：只读拼命令要用的那五个字段，交给 [`launch_command_line`]。
fn build_launch_command(log_path: &Path, fishingbot_logs_dir: &Path) -> String {
    let settings = config::get();

    launch_command_line(
        log_path,
        fishingbot_logs_dir,
        LaunchCommandConfig {
            java_opts: &settings.runtime.java_opts,
            token_file_name: &settings.runtime.token_file,
            filter_regex: &settings.keywords.filter_regex,
            jar: &settings.paths.jar,
            game_config: &settings.paths.game_config,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 启动命令的拼接：JVM 输出过滤后追加进日志（`-logsdir` 指向丢弃目标）。
    ///
    /// 这条命令错了就拉不起机器人，所以按"逐字对照"钉住；期望串里的路径也用 `display()` 拼，
    /// 免得在 Windows 上跑测试时因为分隔符不同而误报。
    #[test]
    fn launch_command_is_built_as_expected() {
        let log_path = Path::new("/srv/bots/1/duck/logs/2026-09-28-01-47-01.log");
        let fishingbot_logs_dir = Path::new("/srv/bots/1/duck/logs/raw");
        let jar = Path::new("/fishingbots/FishingBot.jar");
        let game_config = Path::new("/fishingbots/config.json");

        let command = launch_command_line(
            log_path,
            fishingbot_logs_dir,
            LaunchCommandConfig {
                java_opts: "-Xms32m -Xmx128m",
                token_file_name: "refreshToken",
                filter_regex: "XP|游戏币",
                jar,
                game_config,
            },
        );

        assert_eq!(
            command,
            format!(
                "java -Xms32m -Xmx128m -jar '{}' -nogui -config '{}' -refreshToken 'refreshToken' \
                 -logsdir '{}' 2>&1 | grep --line-buffered -v -E 'XP|游戏币' | tee -a '{}'",
                jar.display(),
                game_config.display(),
                fishingbot_logs_dir.display(),
                log_path.display()
            )
        );
    }

    /// `java_opts` 为空（或只有空白）时不能在 `java` 与 `-jar` 之间留出多余空格。
    #[test]
    fn empty_java_opts_leaves_no_gap() {
        let command = launch_command_line(
            Path::new("/tmp/1.log"),
            Path::new("/tmp/raw"),
            LaunchCommandConfig {
                java_opts: "   ",
                token_file_name: "refreshToken",
                filter_regex: "XP",
                jar: Path::new("/tmp/FishingBot.jar"),
                game_config: Path::new("/tmp/config.json"),
            },
        );

        assert!(command.starts_with("java -jar "), "{command}");
    }
}
