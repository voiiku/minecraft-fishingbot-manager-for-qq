//! `/help`：帮助菜单，文本整段来自配置。

use crate::config;
use crate::utils::Report;

/// 返回帮助菜单。
///
/// 内容来自配置的 `text.help`：改文案不需要重新编译；
/// 指令本身能不能被识别仍由 `command_router` 决定。
pub fn help() -> String {
    let mut report = Report::new("帮助菜单");
    report.content = Some(config::get().text.help.clone());

    report.render()
}
