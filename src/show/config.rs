//! `/show config`：返回所有机器人共用的全局配置内容。

use crate::config;
use crate::utils::Report;

/// 所有机器人共用的游戏侧配置文本（`/show config` 读的就是它）。
pub fn global_game_config() -> Result<String, String> {
    let content = std::fs::read_to_string(&config::get().paths.game_config)
        .map_err(|e| format!("读取全局配置失败: {e}"))?;

    let mut report = Report::new("全局配置 (config.json)");
    report.content = Some(content);

    Ok(report.render())
}
