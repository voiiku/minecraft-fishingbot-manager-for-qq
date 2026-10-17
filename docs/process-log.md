# process_log：本进程日志

* 代码：`src/process_log.rs`
* 对外 API：`init_process_log(debug: bool)`（从 crate 根重导出）

## 一、为什么自己装 logger

kovi 的 `kovi::logger::try_set_logger_use_env()` 用 env_logger 只写终端，而本项目还需要能事后翻查的文件
（`journalctl` 会轮转），所以自己实现 `log::Log`，直接依赖 `log` 与 `chrono`（不经过 kovi 的重导出）。
装好之后 kovi、kovi-onebot 与本项目自己的 `log::*` 全部落到这里；本项目不再用 `println!` 打诊断信息，
因此终端与文件内容一致。

## 二、两条输出

| 输出 | 位置 | 用途 |
| --- | --- | --- |
| 终端（stderr） | 前台输出 / `journalctl -u mc-qq-fishbot` | 实时看；systemd 托管时也能追溯 |
| 文件 | `<部署目录>/logs/年-月-日-时-分-秒.log` | 事后翻查（写一行刷一行，`kill -9` 不丢内容） |

每条记录形如 `[2026-09-28 15:30:12] [INFO] 已加载配置: /fishingbots/mc-qq-fishbot.conf.toml`；
纯文本、不带颜色；文件名复用 `utils::bot_log_files::create_timestamped_log_file`（与机器人日志同一套命名），
首行固定是 `--- 启动 mc-qq-fishbot 进程 ---`。

## 三、级别

`RUST_LOG` 优先，但**只认单个级别词**（`off`/`error`/`warn`/`info`/`debug`/`trace`，大小写不敏感）；
没设置或为空时按配置的 `[kovi] debug`；值认不出来（例如 `kovi=debug` 这类 directive）按默认级别处理，
并写一条告警说明。刻意只保留"一个级别"：本项目按级别调试，不按模块调试。

## 四、失败时怎么办

| 情况 | 行为 |
| --- | --- |
| 部署目录不可写（例如二进制在 `/usr/local/bin`、服务以普通用户跑） | **不致命**：退化成只写终端并写一条告警，机器人照常运行 |
| logger 已经被装过（只会出现在调用两次时） | 保留已装的 logger，打印一条说明 |

理由：日志是运维问题，不该演变成"游戏账号上不了线"。

## 五、调用时机

必须在 `Bot::build` 与任何 `log::*` 之前调用，顺序见 `src/main.rs`（`config::load()` → `init_process_log`
→ `Bot::build` → `bot.run()`）。配置没加载成功时还没有日志文件，那几条提示只出现在终端 ——
这是本项目唯一还在用 `println!` 的地方。
