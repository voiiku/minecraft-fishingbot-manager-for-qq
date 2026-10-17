# mc-qq-fishbot

基于 [Kovi](https://github.com/ThriceCola/Kovi)（OneBot V11 框架）与
[FishingBot](https://github.com/MrKinau/FishingBot) 的 Minecraft 自动钓鱼 QQ 机器人：
用 QQ 群聊/私聊指令批量管理多个游戏账号的钓鱼机器人。

特点：

* **开箱即用**：所有可调的东西都在配置文件里，适配自己的游戏服不需要改代码。
* **一个账号一个 screen 会话**：某个账号掉线/重连/崩溃不影响已经在跑的其他 JVM
  （停服务时才整组一起收，见 [部署文档](docs/deployment.md) 的 `KillMode`）。
* **断开即收工**：QQ 一掉线（kovi 报告连接断开，或心跳判定协议端静默死掉）就停止全部机器人、
  释放游戏账号并退出，由 systemd 重新拉起 —— 机器人不会挂着占账号。
  心跳是**强制**的：协议端必须上报 `meta_event.heartbeat`，启动后一直收不到心跳同样会收工退出。
* **权限基于 QQ 号**：目录按 QQ 号创建，管理操作按调用者 QQ 号执行，管理员可跨账号管理。
* **回复不夸大**：`/start`、`/restart` 只回「已发送…命令」（能确认的只是命令发出与会话出现），
  `/stop` 才会回「已成功停止」（停止流程当场复核会话已消失）。
* **不向部署目录之外写文件**：本项目自己生成的东西只在「部署目录」（配置、本进程日志）与「机器人目录」
  （机器人日志、Token）里；唯一例外是 kovi 自己按**当前工作目录**写的 `kovi.conf.toml` / `kovi.plugin.toml`
  （部署时 `WorkingDirectory` 就是部署目录，见 [部署文档](docs/deployment.md)）。

## 一、文档索引

| 文档 | 内容 |
| --- | --- |
| [CONTEXT.md](CONTEXT.md) | **先读这份**：项目概况 + 必须遵守的全局约束与不变量（给下一位接手的人/AI 会话） |
| [docs/architecture.md](docs/architecture.md) | 分层、目录结构、一条指令的完整路径、必须遵守的不变量 |
| [docs/deployment.md](docs/deployment.md) | 环境准备、首次启动、权限模型、systemd、NapCat 心跳、掉线行为、常见问题 |
| [docs/lib.md](docs/lib.md) | 指令入口层：动作枚举、`handle_*`、错误口径、进程收工入口 |
| [docs/command-router.md](docs/command-router.md) | 指令解析、参数预处理、私聊限制、心跳事件注册 |
| [docs/config.md](docs/config.md) | 配置文件位置、加载流程、表结构、字段规则与启动校验 |
| [docs/process-log.md](docs/process-log.md) | 本进程日志：终端 + `<部署目录>/logs/` 落盘、级别口径 |
| [docs/heartbeat.md](docs/heartbeat.md) | 心跳看门狗的判定规则与收工路径 |
| [docs/bot-control.md](docs/bot-control.md) | `/start` `/stop` `/restart` `/pause` `/unpause` `/command` `/log` |
| [docs/bot-manage.md](docs/bot-manage.md) | `/bot add` `/bot del` `/bot rename` `/token` |
| [docs/all-bots-control.md](docs/all-bots-control.md) | `/startall` `/stopall` `/restartall` `/commandall` |
| [docs/show.md](docs/show.md) | `/status` 与 `/show config|mybots|allbots|admins` |
| [docs/general.md](docs/general.md) | `/help` `/ping` `/checkserver` |
| [docs/utils.md](docs/utils.md) | 原语层模块地图（也可以当作代码导航） |

## 二、机器人指令预览

| 指令 | 说明 | 谁可用 |
| --- | --- | --- |
| `/help` | 帮助菜单（整段文本来自配置；群聊与私聊都可用） | 所有人 |
| `/status` | 检查全部运行中机器人的状态 | 所有人 |
| `/ping [地址] [端口]` | 探测游戏服务器（不带参数用 `[ping]` 表里的目标） | 所有人 |
| `/checkserver` | 查看钓鱼服务器自身的资源占用 | 所有人 |
| `/bot add <机器人名>` | 添加机器人目录 | 所有人（落在自己 QQ 目录下） |
| `/bot del <机器人名>` | 删除机器人目录 | 自己 / 管理员 |
| `/bot rename <旧名> <新名>` | 更名（运行中的先停止，不自动重启） | 自己 / 管理员 |
| `/token <机器人名> <token>` | 写入 refreshToken（**仅私聊**；运行中的机器人不可改） | 自己 / 管理员 |
| `/log <机器人名> <行数>` | 读最新日志（**仅私聊**） | 自己 / 管理员 |
| `/start`、`/stop`、`/restart` | 启动 / 停止 / 重启单个机器人 | 自己 / 管理员 |
| `/pause`、`/unpause` | 暂停钓鱼（不下线）/ 恢复钓鱼 | 自己 / 管理员 |
| `/command <机器人名> <指令>` | 向机器人所在游戏服发送指令 | 自己 / 管理员 |
| `/show config` | 查看游戏侧 `config.json` | 所有人 |
| `/show mybots` | 查看自己名下的机器人 | 所有人 |
| `/show allbots` | 查看全服机器人 | 所有人 |
| `/show admins` | 查看管理员名单（QQ 号） | 所有人 |
| `/startall`、`/stopall`、`/restartall`、`/commandall <指令>` | 对全服批量操作 | 仅管理员 |

每条指令的实现与细节在各模块文档里（见上面的索引）。

## 三、快速上手

1. **准备环境**：Linux + `screen` + 一个 Java 运行时（建议 Java 25，headless 版 JRE 即可）
   + 一个已登录的 OneBot V11 协议端（推荐 NapCat）+ FishingBot 的 jar。
2. **放文件**：二进制、`bots/`、`FishingBot-*.jar`、游戏侧 `config.json` 放同一个目录。
3. **第一次启动**：同级目录下没有 `mc-qq-fishbot.conf.toml` 时，程序会按内置模板生成一份
   （内容与仓库里的 `mc-qq-fishbot.conf.example.toml` 一致）、提示修改后**退出**；
   改好（至少 `paths.base_dir`、`paths.jar`、`[onebot]`、`[admins]`）再启动一次，
   然后在群里发 `/help` 即可。

完整步骤（目录布局、systemd 单元、NapCat 心跳、掉线行为、常见问题）见
[部署与运维](docs/deployment.md)。

## 四、模块地图

```
src/
├── main.rs                进程入口：装配 kovi、运行、断开即收工退出
├── lib.rs                 指令入口层：动作枚举 + handle_* + 收工入口
├── command_router.rs      kovi 插件：指令解析、分发、心跳事件注册
├── heartbeat_event.rs     OneBot 心跳事件：认 meta_event.heartbeat 载荷
├── heartbeat.rs           心跳看门狗：判定窗口与收工判定
├── process_log.rs         本进程日志：终端与 <部署目录>/logs/时间戳.log 同步写
├── config.rs              外置配置：加载、校验、访问（schema/parse/validate/template）
├── bot_control.rs         单机器人控制指令（start/stop/restart/pause/unpause/command/log）
├── bot_manage.rs          账号管理指令（add/del/rename/token）
├── all_bots_control.rs    全服批量指令（startall/stopall/restartall/commandall，仅管理员）
├── show.rs                只读查询指令（status/config/mybots/allbots/admins）
├── general.rs             通用指令（help/ping/checkserver）
└── utils.rs               原语层（bot_dirs/bot_name/bot_log_files/running_bots/report/…）
```

`src/` 下每个文件/子模块都有 `//!` 模块注释，说明「这个文件负责什么、为什么这么写」；
代码怎么读、从哪读起见 [架构与模块地图](docs/architecture.md)。

日志有两条线，都是「每次运行一个新文件、时间戳命名、边跑边写」：本进程日志在
`<部署目录>/logs/`（终端里同时有一份），机器人日志在 `<机器人目录>/<runtime.logs_dir>/`
（只含过滤后的内容，`/log` 读它）。FishingBot 自己的日志指向 `/dev/null`，不占磁盘。
细节见 [utils.md 第七节](docs/utils.md#七日志有两条线) 与 [process-log.md](docs/process-log.md)。

## 五、二次开发

```bash
cargo check --all-targets          # 默认目标是 x86_64-unknown-linux-musl（静态链接）
cargo build --release              # 产物：target/x86_64-unknown-linux-musl/release/mc-qq-fishbot
cargo fmt
cargo test --lib                   # 库的回归测试（配置 schema/校验、心跳判定）
cargo test --bins                  # 插件入口的回归测试（心跳载荷识别）
```

* 本项目默认编译为 musl，需要先执行 `rustup target add x86_64-unknown-linux-musl`。
* 想在本机快速验证语法可以覆盖目标：`cargo check --target 编译目标`。
* 在 Windows 主机上跑测试要显式换目标：`cargo test --lib --target x86_64-pc-windows-msvc`
  （musl 的测试二进制在 Windows 上执行不了；Linux 上直接跑默认目标即可）。
* 配置相关的改动请同步更新 `mc-qq-fishbot.conf.example.toml`，否则 `cargo test` 会失败。
* 心跳事件类型是本项目自己实现的（二进制 crate 的 `src/heartbeat_event.rs` 里的 `HeartbeatEvent`）：
  kovi-onebot 只导出了 `lifecycle` 这类元事件、没有心跳；kovi 会把驱动收到的事件原样交给**每个**
  已注册事件类型的 `Event::de`，所以自定义一个类型即可收到心跳，不需要为它单开连接或轮询 API。
  它的 `de` 除了认出心跳，还把载荷里的 `status.online` 交给看门狗
  （解析在 `heartbeat_event::heartbeat_report`，判定规则见 [heartbeat.md](docs/heartbeat.md)）。

### 命名约定（全项目统一）

* 动作函数用动词短语：`start_bot`、`create_bot_dir`、`create_bot_log_file`、`send_command_to_bot`。
* 取返回值用名词：`token_ready_bot_dir_entries`、`admins_list`、`running_bots_status`、`latest_log_path`，**没有 `get_` 前缀**。
* 转换用 `to_` 前缀：可能失败的转换返回 `Result`，`Err` 是可直接回复给用户的文案 ——
  `to_clean_bot_name`（`str` 的扩展方法，`&str` / `String` 都能直接调）、`to_owner_qq`。
* 构造一个类型时：已知的事实用 `new`，需要去磁盘找的用 `find_*`
  （`BotDirEntry::new` 已知路径 / `BotDirEntry::find_for_caller` 只有名字）。
* 判断用 `is_` / `has_` 前缀：`is_admin`、`is_screen_session_running`、`is_bot_name_taken`。
* **变量名要带上下文与形态**：列表叫 `*_names` / `*_entries` / `*_paths`（例如 `running_bot_names`、
  `not_running_bot_entries`），计数叫 `*_count`（例如 `running_bot_count`）——
  不允许出现裸 `count`、`list`、`data`、`arr` 这类看不出内容的名字，也不用自制简称
  （单位后缀 `_ms`、`_secs` 与一行闭包里的 `|e|` 属于允许的常规写法）。
* 命令路由（`command_router`）只认指令与参数，业务动作全部落在库里，接口统一 `handle_*`。
* 参数名要标形态：从用户文本直接拿到的叫 `raw_*`，清洗过的叫 `clean_*` ——
  同一份文本全链路只清洗一次（路由层只切分、不清洗）。
* 枚举与处理函数跟模块一一对齐：`bot_control` → `BotControlAction` + `handle_bot_control`；
  `bot_manage` → `BotManageAction` + `handle_bot_manage`；`show` → `ShowAction` + `handle_show`；
  `all_bots_control` → `AllBotsControlAction` + `handle_all_bots_control`（管理员专属的全服批量动作）。
