# 架构与模块地图

本文说明「代码分几层、每层负责什么、一条 QQ 消息是怎么落到 screen 会话上的」。
按主题查文档见 [README](../README.md) 的文档索引。

## 一、分层

```
QQ 消息
   │
   ▼
command_router（kovi 插件入口）     认指令 + 洗参数 + 数参数个数，不查权限、不碰业务
   │  parse_command → handle_*
   ▼
lib.rs（指令入口层）               枚举动作 + 批量指令的权限校验 + Result → 回复文本
   │
   ▼
业务模块                            每条指令一个文件
bot_control / bot_manage / all_bots_control / show / general
   │
   ▼
utils（原语层）                     目录、会话、日志、报告等可复用能力
   │
   ▼
screen 会话里的 JVM（FishingBot）
```

**术语**：`command_router` 是**插件入口**（认指令、切分参数、数参数个数，不查权限、不清洗）；
`lib.rs` 的 `handle_*` 是**指令入口层**（动作枚举 → 业务函数，批量指令的权限在这一层查），
下面简称**路由层**；提到**业务层**时指 `bot_control` / `bot_manage` / `show` / `all_bots_control`。

**参数怎么往下传**：插件入口只切分消息、把原始片段原样透传（`raw_*`），谁真正要用就由谁清洗一次 ——
机器人名的清洗规则是"目录命名口径"（`ToCleanBotName::to_clean_bot_name`，非法名字报错而不是过滤），
由 `BotDirEntry` 在构造条目时调用；游戏内指令片段由 `bot_control::command::join_bot_command` 拼一次，见
[command-router.md 第三节](command-router.md)。

另外两条与指令无关的链路：

* `main.rs` —— 进程生命周期：装配 kovi、跑 `bot.run()`、连接断开时收工退出。
* `heartbeat` —— 心跳看门狗：靠协议端上报的心跳发现「WebSocket 静默死掉」，超时后走同一条收工路径。

## 二、目录结构

```
src/
├── main.rs                     进程入口：装配 kovi、运行、断开即收工退出
├── lib.rs                      指令入口层：动作枚举 + handle_* + 收工入口
├── command_router.rs           kovi 插件：指令解析、参数预处理、分发
├── heartbeat_event.rs          OneBot 心跳事件：认 meta_event.heartbeat 载荷
├── heartbeat.rs                心跳看门狗：判定窗口与收工判定
├── process_log.rs              本进程日志：终端（stderr）+ <部署目录>/logs/时间戳.log
├── config.rs                   外置配置的加载、校验与访问（子模块见下）
│   ├── schema.rs               配置结构体（与 toml 一一对应）
│   ├── parse.rs                TOML 反序列化 + 路径展开
│   ├── validate.rs             启动校验
│   └── template.rs             首次启动生成配置模板
├── bot_control.rs              单机器人控制指令
│   ├── start.rs  stop.rs  restart.rs
│   └── pause.rs  unpause.rs  command.rs  log.rs
├── bot_manage.rs               账号管理指令
│   └── add.rs  del.rs  rename.rs  token.rs
├── all_bots_control.rs         全服批量指令（仅管理员）
│   └── startall.rs  stopall.rs  restartall.rs  commandall.rs
├── show.rs                     只读查询指令
│   └── status.rs  config.rs  mybots.rs  allbots.rs  admins.rs
├── general.rs                  通用指令
│   └── help.rs  ping.rs  checkserver.rs
└── utils.rs                    原语层
    ├── bot_dirs.rs             机器人目录：路径、查找、遍历、创建（条目 `BotDirEntry` 内含 Token 状态）
    ├── bot_name.rs             机器人名规范化
    ├── bot_log_files.rs        机器人日志：创建 / 挑选 / 读取，FishingBot 的日志指向 /dev/null
    ├── log_status.rs           从日志文本判定连接/暂停状态
    ├── running_bots.rs         运行中机器人的状态汇总
    ├── screen.rs               会话检测与指令注入
    ├── shutdown.rs             停止会话（`/stop`、`/stopall`、重启的停止阶段、进程收尾）
    ├── startup.rs              启动与重启（四步显式流程）
    ├── uptime.rs               运行时长：从 /proc 读进程已运行秒数 + 格式化成天/时/分
    ├── report.rs               回复报告结构
    └── permission.rs           管理员判断
```

## 三、一条指令的完整路径

以 `/start duck` 为例（群聊，调用者 QQ 为 `123456`）：

1. `command_router::main` 注册群消息回调 → `parse_command(" /start duck")` 得到
   `("/start", ["duck"])`；不是 `/` 开头的消息直接忽略。
2. `handle_group_command` 按指令名分发 → `bot_control_reply(BotControlAction::Start, ...)`：
   只带一个机器人名、参数个数不对就回用法提示。
3. `lib::handle_bot_control` → `bot_control::start::start_bot("duck", 123456)`。
4. `BotDirEntry::find_for_caller` 校验「目录存在 + 调用者有权访问」，并带回归属 QQ 号与 Token 状态：
   先找 `bots/123456/duck`，管理员在自己目录找不到时会再扫一遍全部 QQ 目录。
5. 条目里的 Token 状态确认已配好（`BotTokenStatus::from_bot_dir` 在构造条目时取好），`utils::is_bot_screen_running` 确认未在运行。
6. `utils::start_bot_sessions` 按顺序做三件事：建这次运行的日志文件（时间戳命名）→
   准备 FishingBot 的日志目录（`raw/log0.log` → `/dev/null`）→ 错峰执行
   `screen -dmS fb_duck bash -c '<启动脚本>'`，按 `LaunchTiming::single()` 等一小会儿后
   用一次 `screen -list` 快照复核会话是否真的出现。
7. 结果由 `Report` 组装 → `IntoReplyText::to_reply_text` 转成文本 → 插件回复到群里。

## 四、必须遵守的不变量

* **一个账号一个 screen 会话**：会话名 = `runtime.screen_prefix` + 规范化机器人名，
  唯一来源是 `utils::session_name_for`；只有一份 `screen -list` 解析口径（`utils::screen`）。
* **目录即归属**：机器人目录固定为 `base_dir/bots/QQ号/机器人名`，
  权限边界由 QQ 号那一层决定（`utils::bot_dirs`）。
* **权限分两层，各自只有一处**：
  * 单机器人指令与账号管理指令：**在业务层**由 `BotDirEntry::find_for_caller` 按"目录挂在哪个 QQ 号下"
    判定（普通用户只能碰自己 QQ 目录，管理员可跨目录）；
  * 全服批量指令：**在路由层** `lib::handle_all_bots_control` 用 `is_admin` 判一次，
    `all_bots_control/*` 的函数因此不接 `caller_qq`、也不再各自判断。
  * 插件入口 `command_router` 只数参数个数，不看权限。
* **停止必须彻底**：所有停止入口复用 `utils::stop_sessions`（Ctrl+C → 等 → 强杀 → 复核），
  保证不留占着游戏账号的孤儿 JVM。
* **"运行中不许动"的动作先查会话**：`/token`、`/bot del`、`/bot rename` 在真正动手前都先
  `is_bot_screen_running`；重启流程更严 —— 它在**停止之前**就剔掉缺 Token 的目标
  （重启是先停再起，白停一次会把还在钓鱼的账号弄下线）。
* **断开即收工**：连接断开（`main.rs`）与心跳超时（`heartbeat`）都走
  `lib::stop_all_bots_and_exit`：停全部机器人 → 非 0 退出 → 由 systemd 重新拉起。
* **日志两条线、启动步骤看得见**：机器人日志由 `utils::startup::start_bot_sessions`
  第一步建好（`create_bot_log_file`），会话只往里追加过滤后的输出；FishingBot 自己那份日志写在
  `<runtime.logs_dir>/raw/log0.log`，那是个指向 `/dev/null` 的符号链接（内容丢弃），
  所以磁盘上只有一份日志。「挑哪份日志」只在 `utils::bot_log_files::latest_log_path` 一处决定
  （见 [utils.md 第七节](utils.md#七日志有两条线)）。本进程日志见 [process-log.md](process-log.md)。
* **回复不夸大**：能验证到哪一步就只说到哪一步 —— `/start`、`/restart` 只能确认
  「命令已发出 + 会话已出现」，所以回「已发送…命令」；`/stop` 的流程当场复核会话已消失，
  才可以回「已成功停止」。新增指令按同一条原则选词。
* **Kovi 的边界**：用它的消息/事件/插件框架（`#[kovi::plugin]`、`PluginBuilder`、`kovi::plugins!`、
  `kovi::spawn`）；不用它的 logger 与它自己的配置（`build_bot!` 里那两件事），入口写
  `Bot::build` + `mount_plugin_set`。

## 五、相关文档

| 文档 | 内容 |
| --- | --- |
| [lib.md](lib.md) | 指令入口层与动作枚举 |
| [command-router.md](command-router.md) | 指令解析、参数预处理、分发、心跳事件 |
| [config.md](config.md) | 配置文件 schema、校验与模板 |
| [process-log.md](process-log.md) | 本进程日志（终端 + 部署目录 `logs/`） |
| [heartbeat.md](heartbeat.md) | 心跳看门狗的判定规则 |
| [bot-control.md](bot-control.md) | `/start`、`/stop`、`/restart`、`/pause`、`/unpause`、`/command`、`/log` |
| [bot-manage.md](bot-manage.md) | `/bot add`、`/bot del`、`/bot rename`、`/token` |
| [all-bots-control.md](all-bots-control.md) | `/startall`、`/stopall`、`/restartall`、`/commandall` |
| [show.md](show.md) | `/status`、`/show ...` |
| [general.md](general.md) | `/help`、`/ping`、`/checkserver` |
| [utils.md](utils.md) | 原语层（也可以当作代码导航） |
| [deployment.md](deployment.md) | 部署、systemd、NapCat、常见问题 |
