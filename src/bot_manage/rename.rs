//! `/bot rename`：重命名机器人目录。

use std::time::Duration;
use std::{fs, thread};

use crate::bot_control::stop::stop_bot;
use crate::config;
use crate::utils::{BotDirEntry, ToCleanBotName, is_bot_name_taken, is_bot_screen_running};

/// 重命名机器人（`/bot rename`）。
///
/// 权限在本函数内由 [`BotDirEntry::find_for_caller`] 按目录归属判定（创建者或管理员）。
/// 新旧名字都必须是合法机器人名（[`ToCleanBotName::to_clean_bot_name`]）：非法就直接报错，不再静默过滤。
pub fn rename_bot(
    caller_qq: i64,
    raw_old_bot_name: &str,
    raw_new_bot_name: &str,
) -> Result<String, String> {
    // 目录名一律用清洗后的名字，因此这里先算好，后面的比较与拼接都基于它。
    let clean_old_bot_name = raw_old_bot_name.to_clean_bot_name()?;
    let clean_new_bot_name = raw_new_bot_name.to_clean_bot_name()?;

    if clean_old_bot_name == clean_new_bot_name {
        return Err("错误：新名称与原名称一致，无需修改！".to_string());
    }

    // 定位旧目录（口径＝目录归属）：改别人的机器人只有管理员能做。
    let old_bot_dir = BotDirEntry::find_for_caller(raw_old_bot_name, caller_qq)?.bot_dir_path;

    // 运行中的机器人必须先停止：否则进程会继续往旧目录写数据，改名就会丢日志。
    if is_bot_screen_running(&clean_old_bot_name) {
        stop_bot(&clean_old_bot_name, caller_qq)?;
        // 停止管道已等待会话注销，这里再为文件句柄释放留出缓冲（专用的 rename_settle_ms，
        // 与批量错峰无关，见配置里的注释）。
        thread::sleep(Duration::from_millis(config::get().timing.rename_settle_ms));
    }

    // 重名检查必须在停止之后：停止失败的账号不该改名。
    if is_bot_name_taken(&clean_new_bot_name) {
        return Err(format!(
            "机器人名 '{clean_new_bot_name}' 对应的文件夹已经存在，使用 /show allbots 查看全部机器人"
        ));
    }

    // 新目录与旧目录同属一个 QQ 账号，因此只在父目录里换名字。
    let owner_dir = old_bot_dir
        .parent()
        .ok_or_else(|| "错误：无法获取原机器人目录的父级路径！".to_string())?;
    let new_bot_dir = owner_dir.join(&clean_new_bot_name);

    fs::rename(&old_bot_dir, &new_bot_dir).map_err(|e| format!("重命名失败: {e}"))?;

    Ok(format!(
        "已成功将机器人 '{clean_old_bot_name}' 重命名为 '{clean_new_bot_name}'"
    ))
}
