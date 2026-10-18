//! 启动原语：创建日志、准备 FishingBot 日志目录、启动 screen 会话。
//!
//! 排队、错峰与复核位于 [`super::startup_queue`]；本文件仅负责启动单台机器人，
//! 因此 `/start`、`/startall`、`/bot login` 用的是同一份启动实现。
//!
//! 启动是三个显式步骤：
//!
//! 1. 建这次运行的日志文件（[`super::bot_log_files::create_bot_log_file`]）；
//! 2. 准备 FishingBot 自己的日志目录（[`super::bot_log_files::reset_fishingbot_logs_dir`]：内容丢弃）；
//! 3. `screen -dmS` 后台启动会话（[`launch_screen_session`]）。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config;

use super::bot_dirs::BotDirEntry;
use super::bot_log_files::{bot_logs_dir, create_bot_log_file, reset_fishingbot_logs_dir};
use super::screen::session_name_for;

/// 启动一台机器人：创建日志 → 准备 FishingBot 日志目录 → 启动会话，返回本次日志路径。
///
/// 不进队列、不复核（队列与 `/bot login` 共用这一份实现）；Token 前置检查由调用方做。
/// `/bot login` 会在没有 Token 时调用它，让 FishingBot 自己走微软设备码登录。
pub fn launch_bot_session(bot_dir_entry: &BotDirEntry) -> Result<PathBuf, String> {
    let bot_name = &bot_dir_entry.bot_name;
    let bot_dir_path = &bot_dir_entry.bot_dir_path;

    // 日志建不出来就不拉会话：没有日志就无从查起。
    let log_path = create_bot_log_file(bot_dir_path).map_err(|message| {
        log::error!("机器人 {bot_name} 的日志文件创建失败: {message}");
        format!("机器人 {bot_name}：无法创建日志文件，请检查磁盘空间与目录权限。")
    })?;

    // FishingBot 自己的原始日志写向 /dev/null（磁盘上只留过滤后的那一份）。
    let logs_dir_name = &config::get().runtime.logs_dir;
    let fishingbot_logs_dir = reset_fishingbot_logs_dir(&bot_logs_dir(bot_dir_path, logs_dir_name));

    launch_screen_session(bot_name, bot_dir_path, &log_path, &fishingbot_logs_dir)?;

    Ok(log_path)
}

/// 启动单个机器人的 screen 会话：职责单一，日志与目录准备由 [`launch_bot_session`] 完成。
fn launch_screen_session(
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
    let status = cmd
        .status()
        .map_err(|e| format!("启动机器人 {bot_name} 失败：{e}"))?;
    if !status.success() {
        return Err(format!(
            "启动机器人 {bot_name} 失败：screen 返回 {status}，会话可能已存在。"
        ));
    }

    log::info!(
        "启动机器人 {bot_name} 的 Screen 会话 ({session_name})，日志: {}",
        log_path.display()
    );
    Ok(())
}

/// 启动会话所需的配置取值：集中提供字段，避免参数位置错误。
///
/// 这几个字段散在配置的三个表里（`runtime` / `paths` / `keywords`），
/// 收成一个参数结构体是为了让 [`launch_command_line`] 只依赖"值"、不依赖"配置放在哪"。
#[derive(Debug, Clone, Copy)]
pub struct LaunchCommandConfig<'a> {
    /// `runtime.java_opts`：JVM 参数，未加引号内联进命令。
    pub java_opts: &'a str,
    /// `runtime.token_file`：Token 文件名（相对机器人目录）。
    pub token_file_name: &'a str,
    /// `keywords.filter_regex`：过滤高频日志行的正则（传给 `grep -v -E`）。
    pub filter_regex: &'a str,
    /// `paths.jar`：FishingBot 的 jar。
    pub jar: &'a Path,
    /// `paths.game_config`：FishingBot 的共享配置。
    pub game_config: &'a Path,
}

/// 拼出会话里执行的启动脚本：跑 JVM → 过滤 → 追加进日志文件。
///
/// 纯函数（取值全部由调用方给），所以这条**最危险**的命令能对着固定取值逐字断言 ——
/// 测试位于本文件；读取配置的函数为 [`build_launch_command`]。
///
/// * `-logsdir` 指向 FishingBot 的日志目录（里面是指向 `/dev/null` 的符号链接，内容丢弃）；
/// * `grep --line-buffered -v -E` 过滤高频日志行（**匹配即丢弃**），`--line-buffered` 使日志实时写入，
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
    /// 避免 Windows 路径分隔符差异导致误报。
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

    /// `java_opts` 为空（或仅含空白）时，`java` 与 `-jar` 之间不能产生多余空格。
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
