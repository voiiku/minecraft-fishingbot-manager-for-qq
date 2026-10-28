//! 启动时生成共享游戏配置；旧配置未启用托管时不读写用户的 JSON。

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use kovi::serde_json::{self, Value, json};

use crate::config::{self, FishingBotConfig, Paths};

const TEMPLATE: &str = include_str!("../../fishingbot-config.example.json");

/// 在配置加载成功后、启动任何 FishingBot 之前调用。
pub fn regenerate_fishingbot_config() -> Result<(), String> {
    let config = config::get();
    write_game_config(config.fishingbot.as_ref(), &config.paths)
}

fn write_game_config(fishingbot: Option<&FishingBotConfig>, paths: &Paths) -> Result<(), String> {
    let Some(fishingbot) = fishingbot else {
        return Ok(());
    };

    let destination = &paths.game_config;
    require_safe_destination(&paths.base_dir, destination)?;
    let content = serde_json::to_string_pretty(&build_game_config(fishingbot)?)
        .map_err(|error| format!("生成 FishingBot 配置失败: {error}"))?;
    atomic_write(destination, &format!("{content}\n")).map_err(|error| {
        format!(
            "写入 FishingBot 配置 {} 失败: {error}",
            destination.display()
        )
    })
}

fn managed_values(fishingbot: &FishingBotConfig) -> Vec<(&'static str, Value)> {
    vec![
        ("server.ip", json!(&fishingbot.server_ip)),
        ("server.port", json!(fishingbot.server_port)),
        (
            "server.default-protocol",
            json!(&fishingbot.default_protocol),
        ),
        ("auto.auto-reconnect", json!(fishingbot.auto_reconnect)),
        (
            "auto.auto-reconnect-time",
            json!(fishingbot.auto_reconnect_time),
        ),
        (
            "announces.announce-type-console",
            json!(&fishingbot.announce_type_console),
        ),
        (
            "announces.announce-type-chat",
            json!(&fishingbot.announce_type_chat),
        ),
        (
            "start-text.enabled",
            json!(!fishingbot.start_text.is_empty()),
        ),
        ("start-text.text", json!(&fishingbot.start_text)),
        ("misc.language", json!("CHINESE_SIMPLIFIED")),
    ]
}

fn build_game_config(fishingbot: &FishingBotConfig) -> Result<Value, String> {
    let mut value: Value = serde_json::from_str(TEMPLATE)
        .map_err(|error| format!("FishingBot 配置模板不是合法 JSON: {error}"))?;
    for (path, replacement) in managed_values(fishingbot) {
        let pointer = format!("/{}", path.replace('.', "/"));
        let target = value
            .pointer_mut(&pointer)
            .ok_or_else(|| format!("FishingBot 配置模板缺少托管字段 {path}"))?;
        *target = replacement;
    }
    Ok(value)
}

fn atomic_write(destination: &Path, content: &str) -> std::io::Result<()> {
    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "配置路径没有父目录")
    })?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        ".fishingbot-config-{}-{}.new",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?;
    let result = (|| {
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staging, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(staging);
    }
    result
}

fn require_safe_destination(base: &Path, destination: &Path) -> Result<(), String> {
    let relative = destination
        .strip_prefix(base)
        .map_err(|_| "配置项 paths.game_config 必须位于 paths.base_dir 内".to_string())?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|part| {
            !matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err("配置项 paths.game_config 必须是 base_dir 内的文件路径".to_string());
    }

    // 托管写入不能沿符号链接覆盖其它文件；旧配置不走此分支。
    let mut current = base.to_path_buf();
    for part in relative.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "配置项 paths.game_config 不能经过符号链接: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "检查配置项 paths.game_config {} 失败: {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn staged_paths(tag: &str) -> Paths {
        let base_dir =
            std::env::temp_dir().join(format!("fishingbot-config-{tag}-{}", std::process::id()));
        fs::create_dir_all(&base_dir).unwrap();
        Paths {
            bots_dir: base_dir.join("bots"),
            jar: base_dir.join("FishingBot.jar"),
            game_config: base_dir.join("shared/config.json"),
            base_dir,
        }
    }

    fn fishingbot_example() -> FishingBotConfig {
        FishingBotConfig {
            server_ip: "play.example.com".into(),
            server_port: 25599,
            default_protocol: "1.20.4".into(),
            auto_reconnect: false,
            auto_reconnect_time: 7,
            start_text: vec!["/home fishing".into(), "/balance".into()],
            announce_type_console: "ALL".into(),
            announce_type_chat: "ONLY_BOOKS".into(),
        }
    }

    #[test]
    fn generated_values_match_managed_settings() {
        let fishingbot = fishingbot_example();
        let mut generated = build_game_config(&fishingbot).unwrap();
        assert_eq!(generated["server"]["ip"], "play.example.com");
        assert_eq!(generated["server"]["port"], 25599);
        assert_eq!(generated["server"]["default-protocol"], "1.20.4");
        assert_eq!(generated["auto"]["auto-reconnect"], false);
        assert_eq!(generated["auto"]["auto-reconnect-time"], 7);
        assert_eq!(generated["start-text"]["enabled"], true);
        assert_eq!(
            generated["start-text"]["text"],
            json!(["/home fishing", "/balance"])
        );
        assert_eq!(generated["announces"]["announce-type-console"], "ALL");
        assert_eq!(generated["announces"]["announce-type-chat"], "ONLY_BOOKS");
        assert_eq!(generated["misc"]["language"], "CHINESE_SIMPLIFIED");

        let template: Value = serde_json::from_str(TEMPLATE).unwrap();
        for (path, _) in managed_values(&fishingbot) {
            let pointer = format!("/{}", path.replace('.', "/"));
            *generated.pointer_mut(&pointer).unwrap() = template.pointer(&pointer).unwrap().clone();
        }
        assert_eq!(generated, template, "非托管字段必须逐值保留");
    }

    #[test]
    fn empty_start_text_disables_commands() {
        let mut fishingbot = fishingbot_example();
        fishingbot.start_text.clear();
        let generated = build_game_config(&fishingbot).unwrap();
        assert_eq!(generated["start-text"]["enabled"], false);
        assert_eq!(generated["start-text"]["text"], json!([]));
    }

    #[test]
    fn startup_replaces_old_json_and_writes_complete_template() {
        let paths = staged_paths("managed");
        fs::create_dir_all(paths.game_config.parent().unwrap()).unwrap();
        fs::write(&paths.game_config, b"{\"legacy\":true}").unwrap();
        let fishingbot = fishingbot_example();
        write_game_config(Some(&fishingbot), &paths).unwrap();
        let generated: Value =
            serde_json::from_slice(&fs::read(&paths.game_config).unwrap()).unwrap();
        assert_eq!(generated, build_game_config(&fishingbot).unwrap());
        assert!(generated.get("legacy").is_none());
        assert_eq!(generated["account"]["mail"], "");
        assert_eq!(generated["logs"]["log-count"], 1);
        fs::remove_dir_all(paths.base_dir).unwrap();
    }

    #[test]
    fn legacy_config_is_left_untouched() {
        let paths = staged_paths("legacy");
        fs::create_dir_all(paths.game_config.parent().unwrap()).unwrap();
        fs::write(&paths.game_config, b"user-managed JSON").unwrap();
        write_game_config(None, &paths).unwrap();
        assert_eq!(fs::read(&paths.game_config).unwrap(), b"user-managed JSON");
        fs::remove_file(&paths.game_config).unwrap();
        write_game_config(None, &paths).unwrap();
        assert!(!paths.game_config.exists());
        fs::remove_dir_all(paths.base_dir).unwrap();
    }

    #[test]
    fn rejects_destinations_outside_base() {
        let paths = staged_paths("outside");
        for destination in [
            paths.base_dir.join("../outside.json"),
            paths.base_dir.clone(),
            PathBuf::from("config.json"),
        ] {
            assert!(require_safe_destination(&paths.base_dir, &destination).is_err());
        }
        fs::remove_dir_all(paths.base_dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_destination_and_parent() {
        use std::os::unix::fs::symlink;
        let paths = staged_paths("symlinks");
        let original = paths.base_dir.join("original.json");
        fs::write(&original, b"unchanged").unwrap();
        let destination = paths.base_dir.join("config.json");
        symlink(&original, &destination).unwrap();
        assert!(require_safe_destination(&paths.base_dir, &destination).is_err());
        symlink(&paths.base_dir, paths.base_dir.join("shared")).unwrap();
        assert!(require_safe_destination(&paths.base_dir, &paths.game_config).is_err());
        assert_eq!(fs::read(original).unwrap(), b"unchanged");
        fs::remove_dir_all(paths.base_dir).unwrap();
    }

    #[test]
    fn atomic_replace_preserves_hardlinked_original() {
        let paths = staged_paths("hardlinks");
        fs::create_dir_all(paths.game_config.parent().unwrap()).unwrap();
        fs::write(&paths.jar, b"jar").unwrap();
        fs::hard_link(&paths.jar, &paths.game_config).unwrap();
        atomic_write(&paths.game_config, "new config").unwrap();
        assert_eq!(fs::read(&paths.jar).unwrap(), b"jar");
        assert_eq!(fs::read(&paths.game_config).unwrap(), b"new config");
        fs::remove_dir_all(paths.base_dir).unwrap();
    }
}
