//! 机器人目录相关的工具函数：路径拼接、查找、创建与遍历。
//!
//! 目录布局固定为 `base_dir/bots/QQ号/机器人名`：归属由 QQ 号决定，
//! 机器人目录名由 [`ToCleanBotName::to_clean_bot_name`] 转换后的机器人名决定。
//! 所有查询机器人路径都调用这里的工具函数；而机器人实际运行状态由 [`super::screen`] 判断。

use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config;

use super::bot_name::ToCleanBotName;
use super::permission::is_admin;

/// 枚举机器人目录下 refreshToken 文件的状态，并实现了相应的状态检测/打印方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotTokenStatus {
    /// Token 文件已经存在（目前不检验真实性），可以启动登录。
    Ready,
    /// 还没有 Token 文件
    Missing,
}

impl BotTokenStatus {
    /// 看一个机器人目录里有没有 Token 文件（文件名来自配置 `runtime.token_file`）。
    /// 所有依赖Token状态的判断都以此函数为准。
    pub fn from_bot_dir(bot_dir: &Path) -> Self {
        if bot_dir.join(&config::get().runtime.token_file).exists() {
            Self::Ready
        } else {
            Self::Missing
        }
    }

    /// 是否已经可以启动。
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

impl Display for BotTokenStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => write!(f, "已存在"),
            Self::Missing => write!(f, "待添加"),
        }
    }
}

/// 单个机器人的目录条目结构：归属 QQ 号、机器人名（可能不被清洗）、文件夹路径，以及构造时的 Token 状态。
/// 机器人文件夹路径按 `bots_dir/QQ号/机器人名`拼接，目录的真实性取决于构造需求（有些操作需要即将被创建的目录，而此时还并未创建）
/// 它同时也作为"数据对象"往下传递（启动/停止/状态/列表管道），下游不必再重复拼路径。
#[derive(Debug, Clone)]
pub struct BotDirEntry {
    /// 归属 QQ 号：机器人挂在哪个 QQ 号目录下。
    pub owner_qq: i64,
    /// 机器人文件夹名（就是/bots/QQ号/机器人名 中的机器人名）。
    pub bot_name: String,
    /// 机器人文件夹路径（就是/bots/QQ号/机器人名 这一整个路径）。
    pub bot_dir_path: PathBuf,
    /// Token 文件状态：构造条目时取一次（见 [`BotTokenStatus::from_bot_dir`]）。
    pub bot_token_status: BotTokenStatus,
}
/// 三条构造入口：`new`（已知归属 QQ 号）、`find_for_caller`（要找归属）、`from_dir_name`（磁盘列表）。
impl BotDirEntry {
    /// 目录名 + 归属 QQ 号 → 条目：路径由它们拼出来、Token 状态取一次。
    /// `dir_bot_name` 指"已经是/即将是磁盘上的目录名"，同时为了保证手动建的非法名目录也要能出现在报告明细里，所以不校验。
    fn from_dir_name(dir_bot_name: String, owner_qq: i64) -> Self {
        let bot_dir_path = bot_dir_path(&dir_bot_name, owner_qq);
        let bot_token_status = BotTokenStatus::from_bot_dir(&bot_dir_path);

        Self {
            owner_qq,
            bot_name: dir_bot_name,
            bot_dir_path,
            bot_token_status,
        }
    }

    /// 原始机器人名（QQ传入机器人名的未清洗原文） + 归属 QQ 号，返回条目或异常名字的报错信息。
    pub fn new(raw_bot_name: &str, owner_qq: i64) -> Result<Self, String> {
        Ok(Self::from_dir_name(
            raw_bot_name.to_clean_bot_name()?,
            owner_qq,
        ))
    }

    /// 「原始机器人名 + 调用者」→ 条目：在调用者**有权操作**的范围内找归属。
    /// 非管理员只查自己 QQ 目录；管理员自己目录找不到时扫全服；两者都没有就 `Err`（不区分"不存在"与"无权"）。
    pub fn find_for_caller(raw_bot_name: &str, caller_qq: i64) -> Result<Self, String> {
        let clean_bot_name = raw_bot_name.to_clean_bot_name()?;

        // 先看调用者自己的 QQ 目录，这是普通用户唯一的查找范围。
        if bot_dir_path(&clean_bot_name, caller_qq).exists() {
            return Ok(Self::from_dir_name(clean_bot_name, caller_qq));
        }

        // 管理员自己目录里找不到时，再扫一遍全服。
        if is_admin(caller_qq)
            && let Some(bot_dir_entry) = find_bot_dir_entry_in_all_qq_dirs(&clean_bot_name)?
        {
            return Ok(bot_dir_entry);
        }

        Err(format!(
            "未找到机器人 '{clean_bot_name}' 对应的目录，或你没有权限访问"
        ))
    }
}

/// 「已清洗的机器人名 + 归属 QQ 号」→ 机器人目录的理论路径（`base_dir/bots/QQ号/机器人名`）。
/// 纯拼串：不碰磁盘、也不再清洗（调用方保证名字已经是 [`ToCleanBotName::to_clean_bot_name`] 的产物）。
fn bot_dir_path(clean_bot_name: &str, owner_qq: i64) -> PathBuf {
    config::get()
        .paths
        .bots_dir
        .join(owner_qq.to_string())
        .join(clean_bot_name)
}

/// 把一个目录名解析成 QQ 号；不是合法 QQ 号时返回 `Err`。
/// [`all_bot_dir_entries`] 与 [`is_bot_name_taken`] 都用它判断一个目录是否算机器人的归属目录。
fn to_owner_qq(dir: &Path) -> Result<i64, String> {
    match dir.file_name().and_then(|file_name| file_name.to_str()) {
        Some(dir_name) => dir_name
            .parse::<i64>()
            .map_err(|_| format!("目录名 '{dir_name}' 不是 QQ 号，不算机器人的归属目录")),
        None => Err("目录名不是合法的 UTF-8 文本，无法当 QQ 号用".to_string()),
    }
}

/// 读一个 QQ 目录下的机器人条目（不过滤名字、不排序）。
/// "什么样的子目录算一个机器人目录"只写在这一处：非目录、非 UTF-8 名字都跳过。
fn bot_dir_entries_in_qq_dir(owner_qq: i64, qq_dir: &Path) -> Result<Vec<BotDirEntry>, String> {
    let qq_dir_reader = fs::read_dir(qq_dir).map_err(|e| format!("读取机器人列表失败: {e}"))?;

    Ok(qq_dir_reader
        .flatten()
        .filter_map(|bot_entry| {
            let bot_dir_path = bot_entry.path();
            if !bot_dir_path.is_dir() {
                return None;
            }

            // 非 UTF-8 的目录名没法当机器人名用，跳过。
            let dir_bot_name = bot_dir_path.file_name()?.to_str()?.to_string();
            Some(BotDirEntry::from_dir_name(dir_bot_name, owner_qq))
        })
        .collect())
}

/// 全服所有机器人目录条目，按机器人名排序。
/// 以 `bots/` 下的 QQ 目录为单位遍历：不是 QQ 号、或读不动的目录都跳过，不影响其它账号。
pub fn all_bot_dir_entries() -> Result<Vec<BotDirEntry>, String> {
    let bots_dir = &config::get().paths.bots_dir;
    if !bots_dir.is_dir() {
        return Err("机器人根目录不存在，请检查配置里的 paths.base_dir".to_string());
    }

    let root = fs::read_dir(bots_dir).map_err(|e| format!("读取机器人根目录失败: {e}"))?;

    let mut bot_dir_entries = Vec::new();
    for qq_entry in root.flatten() {
        let qq_path = qq_entry.path();
        if !qq_path.is_dir() {
            continue;
        }

        // 不是 QQ 号的目录（例如手工放进去的其它目录）跳过。
        let Ok(owner_qq) = to_owner_qq(&qq_path) else {
            continue;
        };

        // 单个 QQ 目录读不动时跳过，不影响其它账号。
        if let Ok(qq_bot_dir_entries) = bot_dir_entries_in_qq_dir(owner_qq, &qq_path) {
            bot_dir_entries.extend(qq_bot_dir_entries);
        }
    }

    sort_by_bot_name(&mut bot_dir_entries);
    Ok(bot_dir_entries)
}

/// 在全部 QQ 目录里按名字找机器人条目；找不到返回 `Ok(None)`，根目录读不动时返回 `Err`。
/// 参数必须是 [`ToCleanBotName::to_clean_bot_name`] 转过的名字。
fn find_bot_dir_entry_in_all_qq_dirs(clean_bot_name: &str) -> Result<Option<BotDirEntry>, String> {
    Ok(all_bot_dir_entries()?
        .into_iter()
        .find(|bot_dir_entry| bot_dir_entry.bot_name == clean_bot_name))
}

/// 检查磁盘上是否已存在同名的机器人目录（服务于 `/bot add` 与 `/bot rename` 的重名检查）。
///
/// 参数必须是 [`ToCleanBotName::to_clean_bot_name`] 转过的名字：名字非法时连目录名都不成立，不必来查重名。
/// 名字在磁盘上就是目录名，遍历 QQ 目录时发现同名目录立即返回 true。
pub fn is_bot_name_taken(clean_bot_name: &str) -> bool {
    // 读的是所有 QQ 号的父目录（即 bots），而不是某个 QQ 号的目录。
    let bots_dir = &config::get().paths.bots_dir;
    let Ok(qq_dir_reader) = fs::read_dir(bots_dir) else {
        return false;
    };

    // 只看真正算数的归属目录（与 [`all_bot_dir_entries`] 同一判据），再查里面有没有同名子目录。
    qq_dir_reader.flatten().any(|qq_entry| {
        let qq_path = qq_entry.path();
        qq_path.is_dir() && to_owner_qq(&qq_path).is_ok() && qq_path.join(clean_bot_name).is_dir()
    })
}

/// 为调用者创建机器人数据目录（始终落在调用者自己的 QQ 目录下），成功时返回建出来的条目。
///
/// 名字合法性在 [`BotDirEntry::new`] 里（清洗一次，非法就 `Err`），全服重名见 [`is_bot_name_taken`]；
/// 路径由调用者 QQ 号推导，调用者写不到别人的目录；目录已存在但名字未被占用时视为成功。
pub fn create_bot_dir(raw_bot_name: &str, caller_qq: i64) -> Result<BotDirEntry, String> {
    let bot_dir_entry = BotDirEntry::new(raw_bot_name, caller_qq)?;

    if is_bot_name_taken(&bot_dir_entry.bot_name) {
        return Err(format!(
            "机器人名 '{}' 已被占用。如需多个 QQ 号共享同一个游戏账号，请更换机器人名（机器人名相当于昵称，实际登录角色由 Token 决定）",
            bot_dir_entry.bot_name
        ));
    }

    fs::create_dir_all(&bot_dir_entry.bot_dir_path)
        .map_err(|e| format!("创建机器人目录失败: {e}"))?;

    Ok(bot_dir_entry)
}

/// 全服**已配好 Token** 的机器人目录条目，按机器人名排序。
/// 兜底收尾类操作（`/stopall`、进程退出）用 [`all_bot_dir_entries`]：那些场景要连孤儿进程一起收。
pub fn token_ready_bot_dir_entries() -> Result<Vec<BotDirEntry>, String> {
    let mut bot_dir_entries = all_bot_dir_entries()?;
    bot_dir_entries.retain(|bot_dir_entry| bot_dir_entry.bot_token_status.is_ready());

    Ok(bot_dir_entries)
}

/// 某个 QQ 号**名下**的机器人目录条目，按机器人名排序；该 QQ 目录不存在时返回空列表。
/// 参数叫 `query_qq`（不是 owner_qq）：将来会有"查别人的机器人列表"这种展示场景。
pub fn bot_dir_entries_of_qq(query_qq: i64) -> Result<Vec<BotDirEntry>, String> {
    let query_dir = config::get().paths.bots_dir.join(query_qq.to_string());
    if !query_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut bot_dir_entries = bot_dir_entries_in_qq_dir(query_qq, &query_dir)?;
    sort_by_bot_name(&mut bot_dir_entries);

    Ok(bot_dir_entries)
}

/// 按「Token 是否就绪」把**一批**目录分组：返回 `(已配好, 缺 Token)`。
///
/// 判据就是条目里的 `bot_token_status`；把"被过滤掉的那一半"交回调用方，
/// 是为了 `/startall` 能把缺 Token 的账号列进跳过明细，而不是静默过滤。
pub fn split_bot_entries_by_token_status(
    bot_entries: &[BotDirEntry],
) -> (Vec<BotDirEntry>, Vec<BotDirEntry>) {
    bot_entries
        .iter()
        .cloned()
        .partition(|bot_dir_entry| bot_dir_entry.bot_token_status.is_ready())
}

/// 按机器人名排序；[`all_bot_dir_entries`] 与 [`bot_dir_entries_of_qq`] 共用同一份排序口径。
fn sort_by_bot_name(bot_dir_entries: &mut [BotDirEntry]) {
    bot_dir_entries.sort_unstable_by(|left, right| left.bot_name.cmp(&right.bot_name));
}
