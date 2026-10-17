//! 解析配置文本：TOML 反序列化 + 路径展开。
//!
//! 只做"把文本变成结构体"这件事，取值是否合理交给 [`super::validate`]，
//! 这样"解析失败"和"配置不合理"是两类清晰的报错。

use std::path::Path;

use super::schema::{Config, FileConfig};

/// 解析配置文本：字段缺失、类型不符或名字拼错时，错误信息里会带 TOML 给出的位置；
/// 路径形态（`base_dir` 绝对、`jar` / `game_config` 相对）也在这里一并校验。
pub(super) fn parse(text: &str, path: &Path) -> Result<Config, String> {
    let file: FileConfig = toml::from_str(text).map_err(|e| {
        format!(
            "解析配置文件 {} 失败（字段缺失、类型不符或名字拼错都会在这里报出）:\n{e}",
            path.display()
        )
    })?;

    file.into_config()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 示例配置能被完整解析成运行时结构，并且路径按规则展开好了。
    ///
    /// 只在本项目真正部署的平台（Linux）上跑：示例里的 `base_dir = "/fishingbots"` 在 Windows 上
    /// 不算绝对路径（Windows 需要盘符），校验会（正确地）拒绝它。
    #[cfg(unix)]
    #[test]
    fn example_config_parses() {
        let text = include_str!("../../mc-qq-fishbot.conf.example.toml");
        let path = Path::new("/srv/fishbot/mc-qq-fishbot.conf.toml");
        let config = parse(text, path).expect("示例配置应当能解析");

        assert!(!config.ping.default_ping_host.trim().is_empty());
        assert!(config.ping.default_ping_port > 0);
        assert!(config.ping.timeout_ms > 0);
        assert!(!config.keywords.filter_regex.trim().is_empty());
        assert!(!config.text.help.trim().is_empty());

        // bots 目录一定派生在 base_dir 之下；jar 与 game_config 也都拼成了完整路径。
        assert_eq!(config.paths.bots_dir, config.paths.base_dir.join("bots"));
        assert!(!config.paths.jar.as_os_str().is_empty());
        assert!(!config.paths.game_config.as_os_str().is_empty());
    }

    /// 报错信息里要带上出问题的字段，方便照着改。
    #[test]
    fn parse_error_mentions_field() {
        let text = "[ping]\nhost = \"example.com\"\nport = \"不是端口\"\n";
        let err = parse(text, Path::new("mc-qq-fishbot.conf.toml")).unwrap_err();

        assert!(
            err.contains("mc-qq-fishbot.conf.toml"),
            "报错应指出文件名: {err}"
        );
        assert!(err.contains("port"), "报错应指出字段名: {err}");
    }
}
