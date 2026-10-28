//! `/token`：写入机器人的 refreshToken 文件（仅私聊）。

use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use crate::config;
use crate::utils::{BotDirEntry, StartupQueue, refusal_if_running};

/// 写入机器人的 refreshToken 文件（`/token`，仅私聊）。不校验 Token 有效性。
///
/// 权限在本函数内由 [`BotDirEntry::find_for_caller`] 按目录归属判定（管理员可跨目录）。
/// `raw_token` 是**插件入口原样透传的值**（切分时已去除首尾空白），写入前不再修改。
///
/// 运行中的账号**禁止改 Token**：正在跑的 JVM 只在启动时读一次 Token 文件，改文件对它没有任何
/// 影响，却容易让人以为"已经换好了"。要换就先 `/stop`（与 `/bot del`、`/bot rename` 同一条规矩）。
pub fn update_bot_token_file(
    raw_bot_name: &str,
    raw_token: String,
    caller_qq: i64,
) -> Result<String, String> {
    if raw_token.trim().is_empty() {
        return Err("Token 不能为空。".to_string());
    }
    // 定位目录：存在性与权限一起判定（口径＝目录归属），Token 只会落到调用者有权访问的目录里。
    let bot_dir_entry = BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?;

    if StartupQueue::bot_in_queue(&bot_dir_entry.bot_name) {
        return Err(format!(
            "机器人 '{raw_bot_name}' 已在启动队列中；请先执行 /stop {raw_bot_name}，再更新 Token。"
        ));
    }

    if let Some(refusal) = refusal_if_running(raw_bot_name, "更新 Token") {
        return Err(refusal);
    }

    let token_file_path = bot_dir_entry
        .bot_dir_path
        .join(&config::get().runtime.token_file);

    // 内容原样写入，文件权限仅允许所有者读写。
    write_token_file(&token_file_path, &raw_token)
        .map_err(|e| format!("更新机器人 '{raw_bot_name}' 的 Token 文件失败：{e}"))?;

    let message = format!("已更新机器人 '{raw_bot_name}' 的 Token 文件。");
    log::info!("{message}");
    Ok(message)
}

fn write_token_file(path: &Path, token: &str) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.set_len(0)?;
    file.write_all(token.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_file_create_and_overwrite_preserve_content_and_privacy() {
        let dir =
            std::path::PathBuf::from("target").join(format!("manual-token-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("refreshToken");
        write_token_file(&path, "first token").unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_token_file(&path, "new token\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new token\n");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_token_is_rejected_before_directory_lookup() {
        for token in ["", " ", "\n\t"] {
            assert_eq!(
                update_bot_token_file("duck", token.to_string(), 42),
                Err("Token 不能为空。".to_string())
            );
        }
    }
}
