//! 配置文件模板：首次启动、配置还没写的时候自动生成一份。
//!
//! 模板内容直接复用仓库根的 `mc-qq-fishbot.conf.example.toml`：README 里的说明、
//! 程序首次启动生成的模板、`cargo test` 校验的样例，三者是同一个文件，不会互相对不上。

use std::path::Path;

/// 配置模板（即仓库根的示例配置）。
pub(super) const TEMPLATE: &str = include_str!("../../mc-qq-fishbot.conf.example.toml");

/// 配置文件不存在时写出模板，返回是否真的新建了文件（已存在时**绝不覆盖**，返回 `false`）。
///
/// 上一级目录不存在时不擅自创建（可能是权限受限的路径），而是给出可操作的提示；
/// 提示文案由调用方打印，这里只负责生成文件。
pub(super) fn write_if_missing(path: &Path) -> Result<bool, String> {
    if path.exists() {
        return Ok(false);
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty() && !parent.is_dir())
    {
        return Err(format!(
            "配置文件所在目录 {} 不存在，无法生成模板：请确认可执行文件所在目录存在且可写",
            parent.display()
        ));
    }

    std::fs::write(path, TEMPLATE)
        .map_err(|e| format!("生成配置模板 {} 失败: {e}", path.display()))?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 每个用例一个唯一目录，避免并行执行时互相干扰。
    fn temp_file(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mc-qq-fishbot-template-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");

        dir.join("mc-qq-fishbot.conf.toml")
    }

    /// 文件不存在时按模板生成，内容与示例配置一致。
    #[test]
    fn writes_template_when_missing() {
        let path = temp_file("missing");

        let created = write_if_missing(&path).expect("生成模板失败");
        let written = std::fs::read_to_string(&path).expect("读取生成的模板失败");

        assert!(created, "文件不存在时应当新建");
        assert_eq!(written, TEMPLATE, "生成的内容应当与示例配置完全一致");

        let _ = std::fs::remove_dir_all(path.parent().expect("临时文件必有父目录"));
    }

    /// 文件已存在时不做任何改动。
    #[test]
    fn keeps_existing_file() {
        let path = temp_file("existing");
        std::fs::write(&path, "使用者自己写的配置").expect("写入测试文件失败");

        let created = write_if_missing(&path).expect("已存在的文件不应报错");
        let content = std::fs::read_to_string(&path).expect("读取失败");

        assert!(!created, "文件已存在时不应重建");
        assert_eq!(content, "使用者自己写的配置", "不应覆盖使用者的配置");

        let _ = std::fs::remove_dir_all(path.parent().expect("临时文件必有父目录"));
    }

    /// 上一级目录不存在时给出可操作的提示，而不是 panic。
    #[test]
    fn missing_parent_dir_is_error() {
        let path = std::env::temp_dir()
            .join(format!(
                "mc-qq-fishbot-template-absent-{}",
                std::process::id()
            ))
            .join("mc-qq-fishbot.conf.toml");

        let err = write_if_missing(&path).expect_err("目录不存在时应当报错");

        assert!(err.contains("不存在"), "报错应说明目录不存在: {err}");
    }
}
