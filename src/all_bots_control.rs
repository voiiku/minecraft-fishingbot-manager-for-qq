//! 面向全体机器人的批量指令：`/startall`、`/stopall`、`/restartall`、`/commandall`。
//!
//! 权限**不在本模块**：这四个函数由路由 [`crate::handle_all_bots_control`] 调用，它在调用前
//! 已经用 `is_admin(caller_qq)` 拦过一道（所以本模块的函数一律不接 `caller_qq`、也不自查权限）。
//! 本模块只做「调用批量流程 + 拼装报告」，流程本身都在 `utils` 里
//! （例如批量停止统一走 [`crate::utils::stop_all_bot_sessions`]），保证各入口行为一致。

pub mod commandall;
pub mod restartall;
pub mod startall;
pub mod stopall;
