//! 单个机器人的控制指令：`/start`、`/stop`、`/restart`、`/pause`、`/unpause`、`/command`、`/log`。
//!
//! 每个子模块对应一条指令，会话操作都复用 `utils` 里的原语。
//!
//! 权限**在每个子模块内部校验**（不在路由层）：由 `utils::BotDirEntry::find_for_caller` 按
//! "目录挂在哪个 QQ 号下"判定 —— 普通用户只能操作自己 QQ 目录里的机器人，管理员可跨目录。
//! 插件入口 `command_router` 只数参数个数、不看权限；这一点与 `/startall` 那批批量指令相反
//! （批量的权限在路由层 `handle_all_bots_control` 判一次）。

pub mod command;
pub mod log;
pub mod pause;
pub mod restart;
pub mod start;
pub mod stop;
pub mod unpause;
