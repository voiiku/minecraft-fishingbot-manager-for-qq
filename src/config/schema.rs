//! 配置文件的数据结构：与 `mc-qq-fishbot.conf.toml` 一一对应。
//!
//! 这里只描述"配置长什么样"：读文件见 [`super::parse`]，取值校验见 [`super::validate`]。
//! 字段默认全部必需（少写一个就在解析时报错，`deny_unknown_fields` 让拼错的字段名也立刻暴露）；
//! 唯一的例外是 `timing.heartbeat_timeout_ms`（见 [`DEFAULT_HEARTBEAT_TIMEOUT_MS`]），
//! 给它默认值是为了让**旧配置文件不改一行**也能直接升级运行。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

/// 解析并校验后的配置。
#[derive(Debug)]
pub struct Config {
    /// `/ping` 不带参数时探测的游戏服务器。
    pub ping: PingConfig,
    /// OneBot（QQ 协议端）连接信息。
    pub onebot: OneBotConfig,
    /// 判断机器人状态用的日志关键词。
    pub keywords: KeywordsConfig,
    /// 磁盘路径，相对路径已按配置文件所在目录展开。
    pub paths: Paths,
    /// 机器人运行时参数。
    pub runtime: RuntimeConfig,
    /// 各流程的等待时间。
    pub timing: TimingConfig,
    /// 启动队列的批大小、间隔与复核窗口。
    pub startup_queue: StartupQueueConfig,
    /// 可自定义的提示文案。
    pub text: TextConfig,
    /// 管理员名单（QQ 号 → 名字）。
    pub admins: Admins,
    /// kovi 框架自身的开关。
    pub kovi: KoviFrameworkConfig,
}

/// `/ping` 指令的默认目标服务器与探测超时，只在 `/ping` 不带参数时使用。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PingConfig {
    /// 默认探测的服务器地址，域名或 IP。
    pub default_ping_host: String,
    /// 默认探测的服务器端口。
    pub default_ping_port: u16,
    /// 探测时的连接超时（毫秒）。
    pub timeout_ms: u64,
}

/// 机器人状态判断用的日志关键词，全部取自 FishingBot 自身输出。
///
/// 换 jar 版本或游戏服后需要重新核对，否则状态显示会不准；每项都必填，
/// 由 `config::validate` 在启动时把关。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeywordsConfig {
    /// 成功进入游戏服务器的关键词。
    pub online: String,
    /// 重连提示前缀（此时进程仍在运行）。
    pub reconnect_prefix: String,
    /// 重连提示后缀（此时进程仍在运行）。
    pub reconnect_suffix: String,
    /// 进程结束或被人为中止的关键词。
    pub stopped: String,
    /// 收到 `/pause` 后出现的日志关键词。
    pub pause: String,
    /// 收到 `/unpause` 后出现的日志关键词。
    pub unpause: String,
    /// 过滤垃圾日志用的正则，直接交给 `grep -E`。
    pub filter_regex: String,
}

/// 展开后的文件路径；`jar` 与 `game_config` 均相对 `base_dir` 解析。
#[derive(Debug)]
pub struct Paths {
    /// 数据根目录：机器人目录、jar 与游戏配置的公共父目录。
    pub base_dir: PathBuf,
    /// 机器人目录（`base_dir/bots`），其下按 QQ 号再分一层。
    pub bots_dir: PathBuf,
    /// FishingBot 可执行 jar。
    pub jar: PathBuf,
    /// 所有机器人共用的游戏配置（形如 `config.json`）。
    pub game_config: PathBuf,
}

/// 机器人运行时参数。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    /// 单个机器人 JVM 的启动参数（同时运行的实例较多，内存配额通常要压低）。
    pub java_opts: String,
    /// screen 会话名前缀，最终会话名形如 `fb_机器人名`。
    pub screen_prefix: String,
    /// 机器人目录下保存 refreshToken 的文件名。
    pub token_file: String,
    /// 机器人目录下存放日志的目录名：本程序日志位于该目录。
    ///
    /// FishingBot 自己的日志写在同层 `raw/log0.log`，但那个文件被指向 `/dev/null`（内容丢弃），
    /// 所以磁盘上只有一份日志（见 `utils::bot_log_files`）。
    pub logs_dir: String,
    /// 单次 `/log` 返回的日志行数上限，避免单次输出过多。
    pub max_log_lines: usize,
    /// 单条回复的最大字符数：超过就按行截断，并在末尾注明省略了多少行。
    ///
    /// 默认模板取 3420，约可容纳 `/log` 的 100 行常见日志；实际仍受单行长度影响。
    /// 协议端对单条消息有长度限制（超长会导致发送失败，用户侧无可见响应），
    /// 所以所有报告在 `utils::Report::render` 里统一按这一项收口；
    /// `/show allbots`、批量指令的明细是最可能触到上限的地方。
    pub max_reply_chars: usize,
}

/// 停止、改名与心跳相关的等待参数，单位统一为毫秒。
///
/// 启动节奏（批大小、批内/批间间隔、复核窗口）由 `[startup_queue]` 控制，不在此表。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingConfig {
    /// 停止会话时等待 JVM 优雅退出的时间（毫秒），超时后强制终止。
    pub batch_stop_grace_ms: u64,
    /// 强制终止残留会话后、复核前的缓冲时间（毫秒），确保旧会话完成注销。
    pub batch_stop_settle_ms: u64,
    /// `/bot rename` 停止机器人后、改名前的缓冲时间（毫秒），用于释放文件句柄。
    pub rename_settle_ms: u64,
    /// 心跳判定窗口（毫秒）：超过这么久没收到有效心跳就判定掉线。
    ///
    /// 这是**唯一带默认值**的配置项（见 `DEFAULT_HEARTBEAT_TIMEOUT_MS`）：心跳是发现"静默掉线"
    /// 的唯一手段；缺少该项时旧配置文件无法直接运行，显式写 0 仍然非法（见 `validate`）。
    ///
    /// 它必须大于协议端的心跳间隔（见 `heartbeat` 模块），否则心跳还没到就会判掉线。
    #[serde(default = "default_heartbeat_timeout_ms")]
    pub heartbeat_timeout_ms: u64,
}

/// `timing.heartbeat_timeout_ms` 的默认值（毫秒）。
///
/// 与 NapCat 默认的心跳间隔一致（正向 WebSocket 的 `heartInterval`，30000 毫秒）；
/// 协议端间隔更大时要相应调大这一项。
pub const DEFAULT_HEARTBEAT_TIMEOUT_MS: u64 = 30_000;

/// 启动队列：所有启动请求按批排队启动。
///
/// 与其余配置表相同，整张表及其字段均为必填；缺失或名称拼写错误会在启动时报错。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupQueueConfig {
    /// 一批最多启动几台。默认 1：微软登录按出口 IP 限流，逐台登录较稳定。
    pub max_batch_size: usize,
    /// 批内相邻两台之间至少隔多久（毫秒）。
    pub bot_login_interval_ms: u64,
    /// 批与批之间至少隔多久（毫秒），从上一批被取走的时刻算起。
    pub batch_login_interval_ms: u64,
    /// 等待列表最多容纳几台（不含正在启动的批次）。
    pub max_pending_requests: usize,
    /// 启动后观察会话存活的复核窗口（毫秒）：窗口结束仍在运行的算启动成功。
    pub verify_grace_ms: u64,
}

/// `serde` 的默认值入口：只有 `heartbeat_timeout_ms` 用它（见 [`TimingConfig`]）。
fn default_heartbeat_timeout_ms() -> u64 {
    DEFAULT_HEARTBEAT_TIMEOUT_MS
}

/// 可自定义的文本：目前只有帮助菜单。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextConfig {
    /// `/help` 的完整输出，用 TOML 多行字符串书写。
    ///
    /// 仅影响展示（指令识别由 `command_router` 决定），修改文案无需重新编译。
    pub help: String,
}

/// 管理员名单：QQ 号 → 名字（名字只用于 `/show admins` 展示）。
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct Admins(BTreeMap<String, String>);

impl Admins {
    /// QQ 号是不是管理员。
    pub fn is_admin(&self, qq: i64) -> bool {
        self.0.contains_key(&qq.to_string())
    }

    /// 逐条遍历（按 QQ 号排序，`BTreeMap` 保证顺序稳定）。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(qq, name)| (qq.as_str(), name.as_str()))
    }

    /// 一个人都没配。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// OneBot（QQ 协议端，例如 NapCat）的连接信息。
///
/// 字段与 kovi-onebot 的 `Server` 一一对应；放在本项目配置里，是为了只用**一个配置文件**
/// （kovi 会往自己的 `kovi.conf.toml` 写状态，本项目不读也不写）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneBotConfig {
    /// 服务端地址，域名或 IP。
    pub host: String,
    /// 服务端端口。
    pub port: u16,
    /// 服务端设了 access token 就填在这里，没有就留空。
    pub access_token: String,
    /// 用 wss（TLS）时填 true。
    pub secure: bool,
    /// 反向代理加了路径前缀时才需要改。
    pub path: String,
    /// 单端点模式：所有 API 都走 `/`，此时 `path` 会被忽略。
    pub all_in_one: bool,
}

/// kovi 框架自身的开关。
///
/// 管理员名单已经搬到本文件的 `[admins]`，所以这里只剩框架级选项。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KoviFrameworkConfig {
    /// true 时打印 debug 级别日志（相当于原来的 `[config].debug`）。
    pub debug: bool,
}

/// 配置文件自身的结构，与 `mc-qq-fishbot.conf.toml` 的表一一对应。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileConfig {
    ping: PingConfig,
    admins: Admins,
    onebot: OneBotConfig,
    kovi: KoviFrameworkConfig,
    keywords: KeywordsConfig,
    paths: FilePaths,
    runtime: RuntimeConfig,
    timing: TimingConfig,
    startup_queue: StartupQueueConfig,
    text: TextConfig,
}

impl FileConfig {
    /// 把文件结构转成运行时结构：路径在这里校验形态并派生。
    pub(super) fn into_config(self) -> Result<Config, String> {
        Ok(Config {
            ping: self.ping,
            admins: self.admins,
            onebot: self.onebot,
            kovi: self.kovi,
            keywords: self.keywords,
            paths: self.paths.resolve()?,
            runtime: self.runtime,
            timing: self.timing,
            startup_queue: self.startup_queue,
            text: self.text,
        })
    }
}

/// 配置文件里的路径字段。
///
/// 规则（启动时会校验，写错直接报错）：
///
/// * `base_dir` **必须是绝对路径** —— 它是数据根目录，写清楚最不容易错；
/// * `jar` 与 `game_config` **必须是相对路径**（相对 `base_dir`），它们本来就都在数据根目录下。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FilePaths {
    base_dir: PathBuf,
    jar: PathBuf,
    game_config: PathBuf,
}

impl FilePaths {
    /// 校验路径形态，并展开成运行时路径。
    ///
    /// `base_dir` 必须是绝对路径；`jar` 与 `game_config` 必须写成相对路径，
    /// 展开后分别是 `base_dir/<名字>`；机器人目录固定派生为 `base_dir/bots`。
    fn resolve(self) -> Result<Paths, String> {
        if !self.base_dir.is_absolute() {
            return Err(format!(
                "配置项 paths.base_dir = {} 必须是绝对路径（例如 /fishingbots）",
                self.base_dir.display()
            ));
        }

        let base_dir = self.base_dir;
        let bots_dir = base_dir.join("bots");
        let jar = base_dir.join(require_relative("jar", self.jar)?);
        let game_config = base_dir.join(require_relative("game_config", self.game_config)?);

        Ok(Paths {
            base_dir,
            bots_dir,
            jar,
            game_config,
        })
    }
}

/// 取一个必须写成相对路径的字段。
///
/// 绝对路径会破坏"相对 base_dir"的语义（`jar` 将指向其他位置），因此直接报错。
fn require_relative(field: &str, path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Err(format!(
            "配置项 paths.{field} = {} 必须为相对路径（以 base_dir 为基准）",
            path.display()
        ));
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仓库里那份示例配置必须能被当前 schema 解析：
    /// 修改字段后未同步示例文件（首次启动生成的模板）时，该测试会失败。
    #[test]
    fn example_config_matches_schema() {
        let text = include_str!("../../mc-qq-fishbot.conf.example.toml");
        let file: FileConfig = toml::from_str(text)
            .expect("mc-qq-fishbot.conf.example.toml 与当前配置 schema 不一致，请同步更新示例文件");

        // 示例里的取值是给使用者改的，这里只断言结构与必填字段都在。
        assert!(!file.ping.default_ping_host.trim().is_empty());
        assert!(file.ping.default_ping_port > 0);
        assert!(file.ping.timeout_ms > 0);
        assert!(!file.runtime.screen_prefix.trim().is_empty());
        assert!(file.timing.heartbeat_timeout_ms > 0);
        // 管理员名单、OneBot、kovi 开关这几张表必须出现在示例里，否则用户照抄出来的配置会缺字段。
        assert!(text.contains("[admins]"));
        assert!(text.contains("[onebot]"));
        assert!(text.contains("[kovi]"));
        // 多行字符串里的帮助内容不应被 TOML 的换行处理截断。
        assert!(file.text.help.contains("使用前提示"));
        assert!(file.text.help.contains("/commandall"));
    }

    /// 管理员名单：能按 QQ 号查权限，也能取到名字。
    #[test]
    fn admins_lookup_by_qq() {
        let admins = Admins(BTreeMap::from([(
            "123456789".to_string(),
            "测试管理员".to_string(),
        )]));

        assert!(admins.is_admin(123456789));
        assert!(!admins.is_admin(123456));
        assert!(!admins.is_empty());
        assert!(
            admins
                .iter()
                .any(|(qq, name)| qq == "123456789" && name == "测试管理员")
        );
    }

    /// 少写一个必需的表时必须报错，而不是悄悄用某个默认值。
    #[test]
    fn missing_required_table_is_error() {
        let text = r#"
[ping]
host = "example.com"
port = 25565
"#;

        assert!(toml::from_str::<FileConfig>(text).is_err());
    }

    /// 字段名拼错时必须报错（`deny_unknown_fields`）。
    #[test]
    fn unknown_field_is_error() {
        let text = include_str!("../../mc-qq-fishbot.conf.example.toml")
            .replace("[runtime]", "[runtime]\nunknown_field = 1");

        assert!(toml::from_str::<FileConfig>(&text).is_err());
    }

    /// 构造绝对路径：Linux 使用 `/xxx`，Windows 使用 `C:/xxx`（测试需跨平台运行）。
    fn absolute(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:/{path}"))
        } else {
            PathBuf::from(format!("/{path}"))
        }
    }

    /// `base_dir` 必须是绝对路径、`jar` 与 `game_config` 必须是相对路径（相对 `base_dir`）；
    /// 派生出来的 `bots_dir` 一定在 `base_dir` 之下。
    #[test]
    fn paths_follow_the_absolute_relative_rule() {
        let base_dir = absolute("srv/fishbot/data");
        let paths = FilePaths {
            base_dir: base_dir.clone(),
            jar: PathBuf::from("FishingBot.jar"),
            game_config: PathBuf::from("config.json"),
        }
        .resolve()
        .expect("符合规则的路径应当通过");

        assert_eq!(paths.base_dir, base_dir);
        assert_eq!(paths.bots_dir, base_dir.join("bots"));
        assert_eq!(paths.jar, base_dir.join("FishingBot.jar"));
        assert_eq!(paths.game_config, base_dir.join("config.json"));
    }

    /// 相对路径的 `base_dir` 会被拒绝（提示写绝对路径）。
    #[test]
    fn relative_base_dir_is_rejected() {
        let err = FilePaths {
            base_dir: PathBuf::from("fishingbots"),
            jar: PathBuf::from("FishingBot.jar"),
            game_config: PathBuf::from("config.json"),
        }
        .resolve()
        .expect_err("相对 base_dir 应当报错");

        assert!(err.contains("绝对路径"), "报错应说明要写绝对路径: {err}");
    }

    /// 绝对路径的 `jar` / `game_config` 会被拒绝。
    #[test]
    fn absolute_jar_or_game_config_is_rejected() {
        let base_dir = absolute("srv/fishbot/data");

        let err = FilePaths {
            base_dir: base_dir.clone(),
            jar: absolute("opt/FishingBot.jar"),
            game_config: PathBuf::from("config.json"),
        }
        .resolve()
        .expect_err("绝对 jar 应当报错");
        assert!(err.contains("相对路径"), "报错应说明要写相对路径: {err}");

        let err = FilePaths {
            base_dir,
            jar: PathBuf::from("FishingBot.jar"),
            game_config: absolute("etc/config.json"),
        }
        .resolve()
        .expect_err("绝对 game_config 应当报错");
        assert!(err.contains("game_config"), "报错应指出字段名: {err}");
    }
}
