//! 管理员权限判断。

use crate::config;

/// 判断发起指令的 QQ 号是不是管理员。
///
/// 名单来自 `mc-qq-fishbot.conf.toml` 的 `[admins]`（QQ 号 = 名字），见 [`config::admins`]。
pub fn is_admin(caller_qq: i64) -> bool {
    config::admins().is_admin(caller_qq)
}
