//! 指令入口层：把各业务动作封装成 kovi 路由可直接调用的函数，并统一把错误转换为回复文本。
//!
//! **权限只有两种情况，这里只管其中一种**：批量指令（`/startall` 等）的管理员权限在本层
//! [`handle_all_bots_control`] 判一次；单机器人指令与账号管理指令的权限**不在本层**，
//! 由各业务函数内部按"目录归属哪个 QQ 号"判定（见 [`handle_bot_control`] 的说明）。
//!
//! **参数不在本层清洗**：插件入口只切分消息、把原始片段原样带下来（动作枚举里带着它们），
//! 谁真正要用就由谁清洗一次 —— 机器人名见 `ToCleanBotName::to_clean_bot_name`（`BotDirEntry` 里调用），
//! 游戏内指令片段见 `bot_control::command::join_bot_command`。本层只做动作分发与文案转换。
//!
//! 命名约定见 README 的「开发」一节：动作函数用动词短语、取返回值用名词（不加 `get_` 前缀）、
//! 转换用 `to_`（可能失败就返回 `Result`，`Err` 是可直接回复的文案）、返回 `Result` 不加 `try_`。

use crate::all_bots_control::commandall::send_command_to_all_bots;
use crate::all_bots_control::restartall::restart_all_bots;
use crate::all_bots_control::startall::start_all_bots;
use crate::all_bots_control::stopall::stop_all_bots;
use crate::bot_control::command::send_command_to_bot;
use crate::bot_control::log::read_bot_log;
use crate::bot_control::pause::pause_fishing;
use crate::bot_control::restart::restart_bot;
use crate::bot_control::start::start_bot;
use crate::bot_control::stop::stop_bot;
use crate::bot_control::unpause::unpause_fishing;
use crate::bot_manage::add::add_bot;
use crate::bot_manage::del::delete_bot;
use crate::bot_manage::login::{BotLoginPrompt, begin_bot_login, finish_bot_login};
use crate::bot_manage::rename::rename_bot;
use crate::bot_manage::token::update_bot_token_file;
use crate::general::checkserver::checkserver;
use crate::general::help::help;
use crate::general::ping::ping;
use crate::general::whoami::whoami;
use crate::show::admins::admins_list;
use crate::show::allbots::all_bots_list;
use crate::show::config::global_game_config;
use crate::show::mybots::my_bots_list;
use crate::show::status::status;
// 启动队列类型也导出给指令路由用：它只在「队列忙不忙」这一处读取状态。
pub use crate::utils::StartupQueue;
use crate::utils::{IntoReplyText, is_admin, stop_all_bot_sessions};

mod all_bots_control;
mod bot_control;
mod bot_manage;
pub mod config;
mod general;
mod heartbeat;
mod process_log;
mod show;
mod utils;

// 心跳看门狗对外只暴露这三个：载荷结构体、提交心跳、启动后台任务；判定逻辑都在 `heartbeat` 里。
pub use heartbeat::{HeartbeatReport, note_heartbeat, watch_heartbeat};

// 本进程日志在装配 kovi 之前初始化；此后 kovi 与本库日志同时写入终端和文件。
pub use process_log::init_process_log;

// ============================ 指令动作 ============================

/// 单个机器人的控制动作。
pub enum BotControlAction<'a> {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    /// 向机器人发送游戏内指令：**插件入口原样透传的原始片段**（拼成一条指令与校验在业务函数里做一次）。
    Command(&'a [&'a str]),
}

/// 机器人账号的管理动作。
pub enum BotManageAction {
    Add,
    Delete,
    /// 改名。
    Rename(String),
    /// 更新 refreshToken：插件入口原样透传的值（写入前不改写）。
    Token(String),
}

/// 只读查询动作。
pub enum ShowAction {
    /// `/show status`（与 `/status` 等价）。
    Status,
    /// `/show botconfig`（`/show config` 是别名）。
    BotConfig,
    MyBots,
    AllBots,
    Admins,
}

impl ShowAction {
    /// 解析 `/show` 的子命令名。
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "status" => Self::Status,
            "botconfig" | "config" => Self::BotConfig,
            "mybots" => Self::MyBots,
            "allbots" => Self::AllBots,
            "admins" => Self::Admins,
            _ => return None,
        })
    }
}

/// 面向全体机器人的批量动作（仅管理员）。
pub enum AllBotsControlAction<'a> {
    StartAll,
    StopAll,
    RestartAll,
    /// 向全部运行中的机器人发送同一条指令（原始片段，口径同 [`BotControlAction::Command`]）。
    CommandAll(&'a [&'a str]),
}

// ========================== 对外处理函数 ==========================

/// 返回 `/help` 帮助文本。
pub fn handle_help() -> String {
    help()
}

/// 返回 `/whoami` 身份信息（群聊与私聊都可用）。
pub fn handle_whoami(caller_qq: i64) -> String {
    whoami(caller_qq)
}

/// 启动类指令的进度回执。
///
/// 批量指令只有管理员会收到（非管理员直接走最终结果的拒绝文案）；单机 `/start`、`/restart`
/// 所有调用者都能收到（队列忙时至少让用户知道在排队）。
/// 只读启动队列与配置，不扫描机器人目录。
pub fn handle_startup_receipt(command: &str, caller_qq: i64) -> Option<String> {
    if matches!(command, "/startall" | "/restartall") && !is_admin(caller_qq) {
        return None;
    }

    let pending_bot_count = StartupQueue::pending_bot_count();
    let max_batch_size = StartupQueue::max_batch_size();
    let bot_interval_secs = StartupQueue::bot_login_interval_ms() / 1000;
    let batch_interval_secs = StartupQueue::batch_login_interval_ms() / 1000;

    Some(format!(
        "已提交 {command} 请求；当前等待 {pending_bot_count} 台，每批最多 {max_batch_size} 台，批内间隔 {bot_interval_secs} 秒，批间等待 {batch_interval_secs} 秒；完成后返回结果。"
    ))
}

/// `/bot login` 第一段：启动登录会话并等设备码（权限在业务层按目录归属判定）。
pub fn handle_bot_login_begin(
    raw_bot_name: &str,
    caller_qq: i64,
) -> Result<BotLoginPrompt, String> {
    begin_bot_login(raw_bot_name, caller_qq)
}

/// `/bot login` 第二段：等授权结果、保存 Token 并自动启动。
pub fn handle_bot_login_finish(prompt: BotLoginPrompt) -> String {
    finish_bot_login(prompt)
}

/// 返回 `/status` 运行状态报告。
pub fn handle_status() -> String {
    status().to_reply_text()
}

/// 探测游戏服务器（`/ping`）。
pub async fn handle_ping(host: Option<&str>, port: Option<u16>) -> String {
    ping(host, port).await.to_reply_text()
}

/// 返回钓鱼服务器自身的资源信息（`/checkserver`）。
pub async fn handle_checkserver() -> String {
    checkserver().await.to_reply_text()
}

/// 处理单机器人控制指令（`/start`、`/stop`、`/restart`、`/pause`、`/unpause`、`/command`）。
///
/// 权限**不在本层查**：每个动作函数内部各自调 `utils::BotDirEntry::find_for_caller`，
/// 按机器人目录所属的 QQ 号判定：普通用户只能操作自己目录下的机器人，管理员可跨目录。
/// 本层只做"动作枚举 → 业务函数 + 把 `Result` 转成回复文本"。
pub fn handle_bot_control(
    action: BotControlAction<'_>,
    raw_bot_name: &str,
    caller_qq: i64,
) -> String {
    match action {
        BotControlAction::Start => start_bot(raw_bot_name, caller_qq).to_reply_text(),
        BotControlAction::Stop => stop_bot(raw_bot_name, caller_qq).to_reply_text(),
        BotControlAction::Restart => restart_bot(raw_bot_name, caller_qq).to_reply_text(),
        BotControlAction::Pause => pause_fishing(raw_bot_name, caller_qq).to_reply_text(),
        BotControlAction::Unpause => unpause_fishing(raw_bot_name, caller_qq).to_reply_text(),
        BotControlAction::Command(raw_bot_command_args) => {
            send_command_to_bot(raw_bot_name, caller_qq, raw_bot_command_args).to_reply_text()
        }
    }
}

/// 读取机器人日志（`/log`，仅私聊）。
///
/// 权限同 `/start` 一类：**在业务函数内部**按目录归属判定（见 [`handle_bot_control`]），本层不查。
/// 日志较长时由 `Report::render` 按 `runtime.max_reply_chars` 截断并注明省略行数。
pub fn handle_bot_log(caller_qq: i64, raw_bot_name: &str, line_count: usize) -> String {
    read_bot_log(caller_qq, raw_bot_name, line_count).to_reply_text()
}

/// 处理机器人账号管理指令（`/bot add|del|rename`、`/token`）。
///
/// 权限**不在本层查**：`add` 落在调用者自己的 QQ 目录下，`del` / `rename` / `token` 由业务函数
/// 内部的 `BotDirEntry::find_for_caller` 判定（管理员可跨目录）。
pub fn handle_bot_manage(action: BotManageAction, raw_bot_name: &str, caller_qq: i64) -> String {
    match action {
        BotManageAction::Add => add_bot(raw_bot_name, caller_qq),
        BotManageAction::Delete => delete_bot(raw_bot_name, caller_qq).to_reply_text(),
        BotManageAction::Rename(new_name) => {
            rename_bot(caller_qq, raw_bot_name, &new_name).to_reply_text()
        }
        BotManageAction::Token(raw_token) => {
            update_bot_token_file(raw_bot_name, raw_token, caller_qq).to_reply_text()
        }
    }
}

/// 处理只读查询指令（`/show config|mybots|allbots|admins`）。
///
/// 一律不查权限（谁都能用）：`mybots` 只读调用者自己的 QQ 目录，其余三个读的是全局信息。
pub fn handle_show(action: ShowAction, caller_qq: i64) -> String {
    match action {
        ShowAction::Status => status().to_reply_text(),
        ShowAction::BotConfig => global_game_config().to_reply_text(),
        ShowAction::MyBots => my_bots_list(caller_qq).to_reply_text(),
        ShowAction::AllBots => all_bots_list().to_reply_text(),
        ShowAction::Admins => admins_list(),
    }
}

/// 处理全服批量指令（`/startall`、`/stopall`、`/restartall`、`/commandall`）。
///
/// 这是批量指令的**路由层**（`command_router` 把动作解析出来后就只调到这里）：管理员权限在
/// 这一层用 `is_admin` 判一次，不通过直接返回拒绝文案，`all_bots_control/*` 里的四个动作函数
/// 因此都不接收 `caller_qq`，也不重复校验；避免四份拒绝文案重复实现并保持口径一致。
pub fn handle_all_bots_control(action: AllBotsControlAction<'_>, caller_qq: i64) -> String {
    if !is_admin(caller_qq) {
        return format!("QQ 号 {caller_qq} 不是管理员，无法执行该操作。");
    }

    match action {
        AllBotsControlAction::StartAll => start_all_bots().to_reply_text(),
        AllBotsControlAction::StopAll => stop_all_bots().to_reply_text(),
        AllBotsControlAction::RestartAll => restart_all_bots().to_reply_text(),
        AllBotsControlAction::CommandAll(raw_bot_command_args) => {
            send_command_to_all_bots(raw_bot_command_args).to_reply_text()
        }
    }
}

/// 停止全部机器人（释放游戏账号），供进程退出前调用。
///
/// 不校验管理员权限：这是进程级动作，而非 QQ 指令。
pub fn shutdown_all_bots() -> String {
    match stop_all_bot_sessions() {
        Ok(mut report) => {
            report.set_title("进程退出收尾");
            report.render()
        }
        Err(err) => err,
    }
}

/// 退出：停止全部机器人（释放游戏账号）后**带非 0 退出码退出**，交给 systemd 重新启动。
///
/// "连接断开"（`main.rs` 的 `ExitEvent::FromDrive`）与"心跳超时"（`heartbeat`）共用这一条路径：
/// 非 0 退出码才会命中 systemd 的 `Restart=on-failure`，重新启动就等于重连。
///
/// 用 `std::process::exit` 而不是从 `main` 返回，是因为心跳看门狗是后台任务：
/// 看门狗判定掉线时，`main` 仍阻塞在 `bot.run()` 中，需要立即退出进程。
pub fn stop_all_bots_and_exit(reason: &str) -> ! {
    log::warn!("{reason}");
    log::info!("{}", shutdown_all_bots());
    std::process::exit(1);
}
