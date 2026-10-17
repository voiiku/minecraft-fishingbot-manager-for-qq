//! `/show mybots`：列出某个 QQ 号名下的机器人。

use crate::utils::{Report, bot_dir_entries_of_qq};

/// 列出指定 QQ 号名下的全部机器人，并标注是否已配置 Token。
///
/// 只扫调用者自己的 QQ 目录：别人的机器人不在这里展示（管理员要看全服请用 `/show allbots`）。
/// 目录口径与排序都由 [`bot_dir_entries_of_qq`] 决定，本函数只负责排版。
pub fn my_bots_list(caller_qq: i64) -> Result<String, String> {
    let bot_dir_entries = bot_dir_entries_of_qq(caller_qq)?;
    let mut report = Report::new(format!("QQ账号 {caller_qq} 的机器人列表"));

    if bot_dir_entries.is_empty() {
        report.basics.push("该 QQ 号尚未添加任何机器人".to_string());
        return Ok(report.render());
    }

    // Token 状态由 utils 判断，这里只负责排版。
    report
        .basics
        .push(format!("共 {} 个", bot_dir_entries.len()));
    report
        .basics
        .extend(bot_dir_entries.iter().map(|bot_dir_entry| {
            format!(
                "{} [Token文件: {}]",
                bot_dir_entry.bot_name, bot_dir_entry.bot_token_status
            )
        }));

    Ok(report.render())
}
