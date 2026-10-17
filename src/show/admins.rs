//! `/show admins`：列出管理员名单。

use crate::config;
use crate::utils::Report;

/// 返回管理员名单（`名字 : QQ号`）。
///
/// 名单来自 `mc-qq-fishbot.conf.toml` 的 `[admins]`：QQ 号 = 名字。
pub fn admins_list() -> String {
    let admins = config::admins();

    let mut report = Report::new("管理员名单");
    if admins.is_empty() {
        report.basics.push(
            "还没配置管理员：在 mc-qq-fishbot.conf.toml 的 [admins] 里加一行 QQ号 = \"名字\""
                .to_string(),
        );
        return report.render();
    }

    report
        .basics
        .push(format!("共 {} 个", admins.iter().count()));
    report
        .basics
        .extend(admins.iter().map(|(qq, name)| format!("{name} : {qq}")));

    report.render()
}
