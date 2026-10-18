//! `/stopall`：批量停止所有运行中的机器人（仅管理员）。

use crate::utils::stop_all_bot_sessions;

/// 停止全部运行中的机器人（`/stopall`）。
///
/// 本函数由路由 [`crate::handle_all_bots_control`] 调用，权限在路由层校验
/// 包含 Token 已失效的残留进程；停止流程见 [`stop_all_bot_sessions`]。
pub fn stop_all_bots() -> Result<String, String> {
    let mut report = stop_all_bot_sessions()?;
    report.set_title("全部停止结果");
    Ok(report.render())
}
