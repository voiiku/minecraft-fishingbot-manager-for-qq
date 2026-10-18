//! 项目公共工具函数。
//!
//! 这一层只依赖 `config`（外置配置），不反过来依赖业务模块，
//! 因此 `bot_control` / `all_bots_control` 均可复用。
//!
//! 按用途拆成子模块，这里统一重新导出，外部依旧用 `crate::utils::XXX` 引用。

mod bot_dirs;
mod bot_log_files;
mod bot_name;
// `log_status` 只服务 utils 内部（`running_bots` 用它判定机器人的连接/暂停状态），
// 因此不对外重新导出，避免出现半公开的 API。
mod log_status;
mod permission;
mod report;
mod running_bots;
mod screen;
mod shutdown;
mod startup;
mod startup_queue;
mod uptime;

pub use bot_dirs::*;
pub use bot_log_files::*;
pub use bot_name::*;
pub use permission::*;
pub use report::*;
pub use running_bots::*;
pub use screen::*;
pub use shutdown::*;
pub use startup::*;
pub use startup_queue::*;
pub use uptime::*;
