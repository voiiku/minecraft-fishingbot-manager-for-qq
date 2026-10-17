//! `/bot add`：添加机器人并创建数据目录。

use crate::utils::create_bot_dir;

/// 添加机器人并创建数据目录（`/bot add`）。
///
/// 不查"管理员"这类权限：目录固定建在调用者自己的 QQ 目录下，这就是权限边界；
/// 名字合法性（`ToCleanBotName::to_clean_bot_name`）与重名校验都由 [`create_bot_dir`] 负责，
/// 名字非法时 `Err` 文案直接回给用户，本函数只负责回复文案与后台日志。
pub fn add_bot(raw_bot_name: &str, caller_qq: i64) -> String {
    match create_bot_dir(raw_bot_name, caller_qq) {
        // 回复与后台日志都用磁盘上的真实目录名（清洗后的名字），方便和 `ls` 的结果对上。
        Ok(bot_dir_entry) => {
            log::info!("机器人 {} 创建成功", bot_dir_entry.bot_name);
            format!(
                "为机器人 '{}' 创建目录成功！\n注意：还需为其配置 Token 后才可启动。",
                bot_dir_entry.bot_name
            )
        }
        Err(message) => {
            log::warn!("机器人 {raw_bot_name} 添加失败: {message}");
            format!("机器人 '{raw_bot_name}' 添加失败：{message}")
        }
    }
}
