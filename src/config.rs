//! 外置配置：加载、校验与访问 `mc-qq-fishbot.conf.toml`。
//!
//! 位置固定为**可执行文件同级目录**（不看工作目录、不读环境变量）。
//!
//! * 文件不存在 → 按模板（`mc-qq-fishbot.conf.example.toml`）生成一份、提示后退出；
//! * 字段缺失 / 类型不符 / 取值非法 → 带字段名报错退出，不会带着错误取值运行，也不保留隐式默认值
//!   （只有 `timing.heartbeat_timeout_ms` 有默认值，好让旧配置不改一行也能升级）；
//! * 只在启动时读一次并常驻内存：运行中改配置不生效，重启进程才生效。
//!
//! 管理员名单也在这个文件的 `[admins]`（QQ 号 = 名字），不依赖 kovi 自己那份配置。
//! 对外接口只有 [`load`]、[`get`]、[`path`]、[`admins`]、[`install_dir`]；
//! 内部按职责拆成 `schema`（结构体）/ `parse`（反序列化 + 路径展开）/ `validate`（启动校验）/
//! `template`（模板）。

mod parse;
mod schema;
mod template;
mod validate;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use schema::{
    Admins, Config, KeywordsConfig, KoviFrameworkConfig, OneBotConfig, Paths, PingConfig,
    RuntimeConfig, StartupQueueConfig, TextConfig, TimingConfig,
};

/// 配置文件名：位于可执行文件的同级目录。
pub const CONFIG_FILE_NAME: &str = "mc-qq-fishbot.conf.toml";

/// 进程级配置：启动最早期写入，之后只读。
///
/// 路径与内容放在一起，所以"是否已加载"只有一个判断依据：能取到内容就一定取得到路径。
struct Settings {
    /// 本次启动使用的配置文件路径。
    path: PathBuf,
    /// 解析 + 校验后的配置内容。
    config: Config,
}

static SETTINGS: OnceLock<Settings> = OnceLock::new();

// ============================== 加载入口 ==============================

/// 加载结果。
pub enum LoadOutcome {
    /// 配置文件存在并通过校验，可以继续进入后续流程。
    Loaded(&'static Config),
    /// 原本没有配置文件，已按模板生成一份，提示使用者修改后退出。
    GeneratedTemplate,
}

/// 加载并校验配置。
///
/// 首次运行（配置文件不存在）时先生成模板再返回 [`LoadOutcome::GeneratedTemplate`]。
/// 配置仅在启动时读取一次并常驻内存；重复调用返回已加载内容，不重复读取磁盘。
pub fn load() -> Result<LoadOutcome, String> {
    if let Some(settings) = SETTINGS.get() {
        return Ok(LoadOutcome::Loaded(&settings.config));
    }

    let path = config_path()?;

    // 没有配置文件：写出模板，交给调用方提示使用者改完再启动。
    if template::write_if_missing(&path)? {
        return Ok(LoadOutcome::GeneratedTemplate);
    }

    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取配置文件 {} 失败: {e}", path.display()))?;

    let config = parse::parse(&text, &path)?;
    validate::validate(&config)?;

    // set 失败表示其他线程已完成初始化，沿用已加载的配置。
    let _ = SETTINGS.set(Settings { path, config });
    Ok(LoadOutcome::Loaded(get()))
}

// ============================== 取值入口 ==============================

/// 取已加载的配置。
///
/// # Panics
///
/// 配置尚未加载时 panic，约定见 `settings`。
pub fn get() -> &'static Config {
    &settings().config
}

/// 本次启动使用的配置文件路径（可执行文件同级目录）。
///
/// # Panics
///
/// 配置尚未加载时 panic，约定见 `settings`。
pub fn path() -> &'static Path {
    &settings().path
}

/// 首次启动生成模板后要提示的配置文件路径。
///
/// 此时配置还没加载（模板刚写出来，尚未解析），所以不能走 [`path`]；
/// 这里直接按「可执行文件同级目录」算路径，与 [`load`] 的定位口径一致。
pub fn template_path() -> Result<PathBuf, String> {
    config_path()
}

/// 取管理员名单（`mc-qq-fishbot.conf.toml` 的 `[admins]`）。
pub fn admins() -> &'static Admins {
    &get().admins
}

/// 取进程级配置，只有 [`load`] 成功之后才有。
///
/// # Panics
///
/// 配置尚未加载时 panic。进程入口第一件事就是调用 [`load`]，走到这里还 panic 说明调用顺序写错了。
fn settings() -> &'static Settings {
    SETTINGS
        .get()
        .expect("配置尚未加载：请先在进程入口调用 mc_qq_fishbot::config::load()，再使用其它接口")
}

/// 部署目录：**可执行文件所在目录**（配置文件、kovi 的状态文件、本进程的日志都在这里）。
///
/// 不读取工作目录或环境变量：部署时将二进制与配置放在同一目录即可。
/// systemd 之类的托管也不需要额外设置。
pub fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("无法确定可执行文件位置，因此找不到配置与日志目录: {e}"))?;

    exe.parent().map(PathBuf::from).ok_or_else(|| {
        format!(
            "可执行文件路径 {} 没有上级目录，无法确定配置与日志目录",
            exe.display()
        )
    })
}

/// 定位配置文件：部署目录下的 `mc-qq-fishbot.conf.toml`。
fn config_path() -> Result<PathBuf, String> {
    Ok(install_dir()?.join(CONFIG_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 配置文件位置固定为可执行文件同级目录。
    #[test]
    fn config_path_is_next_to_executable() {
        let path = config_path().expect("测试环境下应当能取到可执行文件路径");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(CONFIG_FILE_NAME)
        );
        assert_eq!(
            path.parent(),
            std::env::current_exe().expect("取可执行文件路径").parent()
        );
    }
}
