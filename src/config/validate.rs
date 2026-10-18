//! 启动校验：只看「写错了就跑不起来」的三类问题，不做详尽检查。
//!
//! 1. **必填项不能为空** —— [`require_non_empty`]；
//! 2. **数值范围** —— 端口不能是 0，条数与时间参数必须大于 0（`ping.timeout_ms`、
//!    `runtime.max_log_lines`、`runtime.max_reply_chars`、`timing.heartbeat_timeout_ms`）；
//! 3. **简单合法性** —— 只查会被视为"单个名字"或拼接到 shell 命令的字符串：
//!    [`require_plain_name`]（不能含空白/斜杠，也不能是 `.` / `..`）、`runtime.java_opts`
//!    不能含 shell 元字符、`keywords.filter_regex` 与两个路径字段不能含单引号。
//!
//! 除此之外一律不管：参数该设多大、地址写哪个、中文名字行不行，都是使用者的判断，
//! 启动时不做推荐或推断。每条报错均为「配置项 + 字段名 + 要求」（由 [`invalid`] 统一生成），按提示修改即可。

use super::schema::{
    Admins, Config, KeywordsConfig, OneBotConfig, Paths, PingConfig, RuntimeConfig,
    StartupQueueConfig, TextConfig, TimingConfig,
};

/// shell 命令里未加引号的内联取值禁止出现的字符（`runtime.java_opts`）。
const SHELL_UNSAFE: &[char] = &['\'', '"', '`', '$', ';', '&', '|', '<', '>', '\n', '\r'];

/// 校验整份配置：一组相关的取值一个函数，调用顺序与 `mc-qq-fishbot.conf.toml` 里的表顺序一致。
pub(super) fn validate(config: &Config) -> Result<(), String> {
    validate_ping(&config.ping)?;
    validate_onebot(&config.onebot)?;
    validate_admins(&config.admins)?;
    validate_keywords(&config.keywords)?;
    validate_paths(&config.paths)?;
    validate_runtime(&config.runtime)?;
    validate_timing(&config.timing)?;
    validate_startup_queue(&config.startup_queue)?;
    validate_text(&config.text)
}

/// 启动队列：批大小与等待容量不能是 0，复核窗口必须大于 0（0 会让复核永远判定失败）。
///
/// 两个间隔允许为 0：本机调试可将间隔设为最短；微软登录限流由使用者自行控制。
fn validate_startup_queue(startup_queue: &StartupQueueConfig) -> Result<(), String> {
    if startup_queue.max_batch_size == 0 {
        return Err(invalid("startup_queue.max_batch_size", "必须大于 0"));
    }
    if startup_queue.max_pending_requests == 0 {
        return Err(invalid("startup_queue.max_pending_requests", "必须大于 0"));
    }
    if startup_queue.verify_grace_ms == 0 {
        return Err(invalid("startup_queue.verify_grace_ms", "必须大于 0"));
    }

    Ok(())
}

/// 拼一条字段级报错：`配置项 <字段> <要求>`。
///
/// 报错格式只写在这一处，各条规则只需要给出字段名与要求。
fn invalid(field: impl std::fmt::Display, requirement: &str) -> String {
    format!("配置项 {field} {requirement}")
}

/// 同 [`invalid`]，但把出问题的取值一起报出来：`配置项 <字段> = <取值> <要求>`。
fn invalid_value(
    field: impl std::fmt::Display,
    value: impl std::fmt::Display,
    requirement: &str,
) -> String {
    format!("配置项 {field} = {value} {requirement}")
}

/// 必填的文本项：`配置项 <字段> 不能为空`。
///
/// "非空"是最常见的一条规则（写空字符串与漏填一样跑不起来），统一在这一点收口。
fn require_non_empty(field: impl std::fmt::Display, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(invalid(field, "不能为空"));
    }

    Ok(())
}

/// 会被视为"单个名字"用的取值（会话名前缀、Token 文件名、日志目录名）。
///
/// 仅拒绝两类会导致实际问题的写法：含**空白或斜杠**（在命令与路径中会被拆分，`../` 还会将文件
/// 写到机器人目录之外），以及 `.` / `..`。中文与 emoji 可正常使用，启动校验不做字符集白名单。
fn require_plain_name(field: impl std::fmt::Display, value: &str) -> Result<(), String> {
    let has_whitespace = value.chars().any(char::is_whitespace);
    let has_separator = value.contains(['/', '\\']);
    let is_dot_name = matches!(value, "." | "..");

    if has_whitespace || has_separator || is_dot_name {
        return Err(invalid(field, "不能包含空格或斜杠，也不能写成 . 或 .."));
    }

    // 此处已确认不含空白，空字符串仅剩长度为零这一种情况，交由非空检查统一处理。
    require_non_empty(field, value)
}

/// `/ping` 的默认目标与探测超时。
fn validate_ping(ping: &PingConfig) -> Result<(), String> {
    require_non_empty("ping.default_ping_host", &ping.default_ping_host)?;
    if ping.default_ping_port == 0 {
        return Err(invalid(
            "ping.default_ping_port",
            "必须是 1-65535 之间的端口号",
        ));
    }
    if ping.timeout_ms == 0 {
        return Err(invalid("ping.timeout_ms", "必须大于 0"));
    }

    Ok(())
}

/// OneBot（QQ 协议端）连接信息。
fn validate_onebot(onebot: &OneBotConfig) -> Result<(), String> {
    if onebot.host.trim().is_empty() {
        return Err(invalid("onebot.host", "不能为空（填 OneBot 服务端的地址）"));
    }
    if onebot.port == 0 {
        return Err(invalid("onebot.port", "必须是 1-65535 之间的端口号"));
    }

    Ok(())
}

/// 管理员名单：空名单是允许的（那就没有管理员，各人管各人的机器人）；
/// 但键写错会让权限**静默失效**，所以非数字的键、空名字都直接拦下来。
fn validate_admins(admins: &Admins) -> Result<(), String> {
    for (qq, name) in admins.iter() {
        if qq.parse::<i64>().is_err() {
            return Err(invalid(
                "admins 的键",
                &format!("{qq:?} 不是 QQ 号：应写成纯数字，例如 \"123456789\" = \"名字\""),
            ));
        }
        if name.trim().is_empty() {
            return Err(invalid("admins", &format!("里 QQ {qq} 的名字不能为空")));
        }
    }

    Ok(())
}

/// 日志关键词：任何一项没填都会让状态显示不准。
///
/// `filter_regex` 会内联进 `grep -E` 的单引号参数里，因此不能包含单引号。
fn validate_keywords(keywords: &KeywordsConfig) -> Result<(), String> {
    let fields = [
        ("online", &keywords.online),
        ("reconnect_prefix", &keywords.reconnect_prefix),
        ("reconnect_suffix", &keywords.reconnect_suffix),
        ("stopped", &keywords.stopped),
        ("pause", &keywords.pause),
        ("unpause", &keywords.unpause),
        ("filter_regex", &keywords.filter_regex),
    ];

    for (field, value) in fields {
        require_non_empty(format!("keywords.{field}"), value)?;
    }

    if keywords.filter_regex.contains('\'') {
        return Err(invalid("keywords.filter_regex", "不能包含单引号"));
    }

    Ok(())
}

/// 磁盘路径：少了 `base_dir` 或 `jar`，每个机器人都起不来。
///
/// 路径形态（`base_dir` 绝对、其它相对）在解析阶段就查过了，这里只管"文件有没有就位"；
/// `game_config` 不检查存在性 —— 它由 FishingBot 自己创建与维护，首次运行时可能才刚生成。
fn validate_paths(paths: &Paths) -> Result<(), String> {
    if !paths.base_dir.is_dir() {
        return Err(invalid_value(
            "paths.base_dir",
            paths.base_dir.display(),
            "不存在或不是目录",
        ));
    }

    if !paths.jar.is_file() {
        return Err(invalid_value(
            "paths.jar",
            paths.jar.display(),
            "不存在或不是文件（相对路径以 base_dir 为基准）",
        ));
    }

    // 两个路径都会以单引号形式拼接到启动命令，路径里出现单引号会破坏命令。
    for (field, path) in [("jar", &paths.jar), ("game_config", &paths.game_config)] {
        if path.to_string_lossy().contains('\'') {
            return Err(invalid(format!("paths.{field}"), "不能包含单引号"));
        }
    }

    Ok(())
}

/// 运行时参数。
///
/// 这些取值会被拼接到 `bash -c` 的启动命令：`java_opts` 未加引号内联，
/// `screen_prefix` / `token_file` / `logs_dir` 会被视为**单个名字**使用（会话名、文件名、目录名），
/// 因此只做最简单的合法性检查（见 [`require_plain_name`]）：
/// `logs_dir = "../x"` 会把日志写到机器人目录外面，`screen_prefix = "fb _"` 会把会话名拆开。
fn validate_runtime(runtime: &RuntimeConfig) -> Result<(), String> {
    if runtime.java_opts.contains(SHELL_UNSAFE) {
        return Err(invalid(
            "runtime.java_opts",
            "不能包含引号、反引号、$、;、&、|、<、> 或换行",
        ));
    }

    for (field, value) in [
        ("screen_prefix", &runtime.screen_prefix),
        ("token_file", &runtime.token_file),
        ("logs_dir", &runtime.logs_dir),
    ] {
        require_plain_name(format!("runtime.{field}"), value)?;
    }

    if runtime.max_log_lines == 0 {
        return Err(invalid("runtime.max_log_lines", "必须大于 0"));
    }
    if runtime.max_reply_chars == 0 {
        return Err(invalid(
            "runtime.max_reply_chars",
            "必须大于 0（否则每条回复都会被截成空消息）",
        ));
    }

    Ok(())
}

/// 心跳看门狗的判定窗口。
///
/// 仅拒绝"必然出问题"的取值：为 0 时**一收到心跳就判定掉线**（随后立即退出、
/// 被 systemd 重新启动），等于把程序变成反复重启的循环。
///
/// 这一项在 schema 里有默认值（默认 30000 毫秒，为的是旧配置不改一行也能升级），
/// 但**显式写 0 仍然按非法处理**：默认值是给"没写"用的，不是给"写错"用的。
fn validate_timing(timing: &TimingConfig) -> Result<(), String> {
    if timing.heartbeat_timeout_ms == 0 {
        return Err(invalid(
            "timing.heartbeat_timeout_ms",
            "必须大于 0（这是心跳判定窗口，单位毫秒）",
        ));
    }

    Ok(())
}

/// 帮助文本。
fn validate_text(text: &TextConfig) -> Result<(), String> {
    require_non_empty("text.help", &text.help)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use crate::config::schema::DEFAULT_HEARTBEAT_TIMEOUT_MS;
    use std::path::{Path, PathBuf};

    /// 示例配置（也就是首次启动生成的模板）。
    ///
    /// 示例值可由使用者修改（占位地址、jar 版本号等），因此测试只依赖其**结构**：
    /// 修改字段时使用 [`set_field`] 按字段名替换，不依赖其当前取值。
    const EXAMPLE: &str = include_str!("../../mc-qq-fishbot.conf.example.toml");

    /// 把 `field = ...` 这一行换成 `field = <value>`（`value` 原样写，例如 `"\"x\""`）。
    fn set_field(text: &str, field: &str, value: &str) -> String {
        let prefix = format!("{field} ");
        let mut done = false;

        text.lines()
            .map(|line| {
                if !done && line.trim_start().starts_with(&prefix) {
                    done = true;
                    format!("{field} = {value}")
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 造一个临时目录，并在里面按示例配置写的 jar 文件名放一个占位文件；
    /// 然后将 `base_dir` 指向该目录（示例使用占位路径，测试需替换）。
    fn staged_example(tag: &str) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "mc-qq-fishbot-validate-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");

        // jar 名称取自示例文件，避免示例版本变化导致测试失败。
        let jar_name = EXAMPLE
            .lines()
            .find_map(|line| line.trim_start().strip_prefix("jar = "))
            .map(|value| value.trim().trim_matches('"').to_string())
            .unwrap_or_else(|| "FishingBot.jar".to_string());
        std::fs::write(dir.join(jar_name), b"jar").expect("写入测试 jar 失败");

        // 路径里不能出现反斜杠：TOML 基本字符串会把 `\` 当转义符。
        let base_dir = dir.to_string_lossy().replace('\\', "/");
        let text = set_field(EXAMPLE, "base_dir", &format!("\"{base_dir}\""));

        (dir, text)
    }

    /// 跑一遍 `load_from` 的核心流程：解析 + 校验。
    fn load_text(text: &str, config_dir: &Path) -> Result<Config, String> {
        let config = parse::parse(text, &config_dir.join("mc-qq-fishbot.conf.toml"))?;
        validate(&config)?;
        Ok(config)
    }

    /// 路径都就位时，示例配置应当通过校验。
    #[test]
    fn example_config_passes_validation() {
        let (dir, text) = staged_example("ok");

        let config = load_text(&text, &dir).expect("示例配置应当通过校验");
        assert_eq!(config.paths.base_dir, dir);
        assert!(config.paths.jar.is_file());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `base_dir` 不存在或不是目录（首次部署最容易犯的错）要报出字段名与原因。
    #[test]
    fn missing_base_dir_is_reported() {
        let (dir, text) = staged_example("missing-base");
        // 此处必须保持绝对路径；否则会先被路径形态校验拦截，无法覆盖后续规则。
        let missing = dir.join("not-there").to_string_lossy().replace('\\', "/");
        let text = set_field(&text, "base_dir", &format!("\"{missing}\""));

        let err = load_text(&text, &dir).expect_err("目录不存在应当报错");

        assert!(err.contains("paths.base_dir"), "报错应指出字段: {err}");
        assert!(err.contains("不存在或不是目录"), "报错应说明原因: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 能破坏启动命令的 `java_opts` 要拦下来。
    #[test]
    fn unsafe_java_opts_is_reported() {
        let (dir, text) = staged_example("java-opts");
        let text = set_field(&text, "java_opts", "\"-Xmx128m; rm -rf /\"");

        let err = load_text(&text, &dir).expect_err("危险字符应当报错");

        assert!(err.contains("runtime.java_opts"), "报错应指出字段: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 会话名前缀中的空白会使 screen 命令参数错位。
    #[test]
    fn unsafe_screen_prefix_is_reported() {
        let (dir, text) = staged_example("screen-prefix");
        let text = set_field(&text, "screen_prefix", "\"fb _\"");

        let err = load_text(&text, &dir).expect_err("空白字符应当报错");

        assert!(
            err.contains("runtime.screen_prefix"),
            "报错应指出字段: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `/log` 的行数上限为 0 时永远看不到日志，必须在启动时就拦下。
    #[test]
    fn zero_max_log_lines_is_reported() {
        let (dir, text) = staged_example("max-log-lines");
        let text = set_field(&text, "max_log_lines", "0");

        let err = load_text(&text, &dir).expect_err("0 应当报错");

        assert!(
            err.contains("runtime.max_log_lines"),
            "报错应指出字段: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `/ping` 的超时为 0 时永远等不到响应，必须在启动时就拦下。
    #[test]
    fn zero_ping_timeout_is_reported() {
        let (dir, text) = staged_example("ping-timeout");
        let text = set_field(&text, "timeout_ms", "0");

        let err = load_text(&text, &dir).expect_err("0 应当报错");

        assert!(err.contains("ping.timeout_ms"), "报错应指出字段: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 心跳判定窗口为 0 会让看门狗一收到心跳就判定掉线（变成反复重启），必须在启动时就拦下。
    #[test]
    fn zero_heartbeat_timeout_is_reported() {
        let (dir, text) = staged_example("heartbeat-timeout");
        let text = set_field(&text, "heartbeat_timeout_ms", "0");

        let err = load_text(&text, &dir).expect_err("0 应当报错");

        assert!(
            err.contains("timing.heartbeat_timeout_ms"),
            "报错应指出字段: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 旧配置里没有 `heartbeat_timeout_ms`（升级前的版本没有这一项）时要用默认值，
    /// 不能因为缺字段就启动失败：老用户下载新二进制后应当"直接就能跑"。
    #[test]
    fn missing_heartbeat_timeout_uses_default() {
        let (dir, text) = staged_example("heartbeat-default");
        let text = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("heartbeat_timeout_ms"))
            .collect::<Vec<_>>()
            .join("\n");

        let config = load_text(&text, &dir).expect("缺这一项时应当用默认值");

        assert_eq!(
            config.timing.heartbeat_timeout_ms,
            DEFAULT_HEARTBEAT_TIMEOUT_MS
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 管理员名单的键必须是 QQ 号：写成其他值会导致权限静默失效。
    #[test]
    fn admin_key_must_be_qq() {
        let admins = admins_from("\"abc\" = \"名字\"");

        let err = validate_admins(&admins).expect_err("非数字键应当报错");

        assert!(err.contains("QQ 号"), "报错应说明键要是 QQ 号: {err}");
    }

    /// 管理员名字不能为空白，否则会影响 /show admins 的可读性。
    #[test]
    fn admin_name_must_not_be_empty() {
        let admins = admins_from("\"123456789\" = \"   \"");

        let err = validate_admins(&admins).expect_err("空名字应当报错");

        assert!(err.contains("名字不能为空"), "报错应说明名字为空: {err}");
    }

    /// 正常名单应当放行，并且能按 QQ 号查到权限。
    #[test]
    fn good_admins_pass() {
        let admins = admins_from("\"123456789\" = \"测试管理员\"\n\"123456\" = \"小号\"");

        assert!(validate_admins(&admins).is_ok());
        assert!(admins.is_admin(123456789));
        assert!(admins.is_admin(123456));
    }

    /// 按真实格式解析管理员名单：`Admins` 字段为私有，测试通过 TOML 解析以贴近实际格式。
    fn admins_from(text: &str) -> Admins {
        toml::from_str(text).expect("测试用的管理员名单应当能解析")
    }
}
