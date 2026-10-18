//! `/whoami`：返回调用者自己的 QQ 号，以及可以直接抄进 `[admins]` 的一行。
//!
//! 存在的意义是解开「先有鸡还是先有蛋」：配置管理员要填 QQ 号，而 QQ 客户端里
//! 找到自己的号码对不熟的人并不直观。

use crate::utils::Report;

/// 生成身份信息（群聊与私聊都可用，不查权限）。
pub fn whoami(caller_qq: i64) -> String {
    let mut report = Report::new("身份信息");
    report.basics.push(format!("QQ 号：{caller_qq}"));
    report
        .basics
        .push("如需添加管理员，请将以下内容写入配置文件的 [admins] 段，并重启进程：".to_string());
    report.content = Some(format!("\"{caller_qq}\" = \"你的名字\""));

    report.render()
}
