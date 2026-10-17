//! 机器人账号管理指令：新增、删除、改名、更新 refreshToken。
//!
//! 权限**在每个子模块内部校验**（不在路由层）：口径同样是"目录归属哪个 QQ 号" ——
//! `add` 落在调用者自己的 QQ 目录下，`del` / `rename` / `token` 由 `utils::BotDirEntry::find_for_caller`
//! 判定（管理员可跨目录）。插件入口 `command_router` 只数参数个数。

pub mod add;
pub mod del;
pub mod rename;
pub mod token;
