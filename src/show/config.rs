//! `/show config`：返回所有机器人共用的全局配置内容。

use crate::config;
use crate::utils::Report;

/// 所有机器人共用的游戏侧配置文本（`/show config` 读的就是它）。
pub fn global_game_config() -> Result<String, String> {
    let game_config_path = &config::get().paths.game_config;
    let content =
        std::fs::read_to_string(game_config_path).map_err(|e| format!("读取配置失败：{e}"))?;

    // 标题使用实际配置文件名，避免修改 paths.game_config 后显示与实际文件不一致。
    let game_config_file_name = game_config_path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("config.json");
    let mut report = Report::new(format!("配置（{game_config_file_name}）"));
    report.content = Some(content);

    Ok(report.render())
}
