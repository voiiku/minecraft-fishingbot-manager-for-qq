# CONTEXT：项目概况与必须记住的事

本文件是**给下一位接手的人（或 AI 会话）的最短上下文**：上面几行说清项目是什么，
下面详细列清「必须关注、不能被忘记」的硬约束。**改动代码前先读这里，改完如有新约定请更新这里。**
细节文档：[README](README.md)（速览）→ [docs/](docs/)（每份对应一个模块）。

## 一、项目概况

- 干什么：用 QQ 群/私聊指令，批量管理多个 Minecraft 账号的
  [FishingBot](https://github.com/MrKinau/FishingBot) 自动钓鱼实例。
- 技术栈：Rust（edition 2024）+ [Kovi](https://github.com/ThriceCola/Kovi)（OneBot V11 框架）
  - `screen` + JVM；默认目标 `x86_64-unknown-linux-musl`，**只面向 Linux 部署**。
- 进程模型：一个机器人 = 一个 `screen` 会话（会话名 = `runtime.screen_prefix` + 机器人名）。
- 一个进程只管一个 QQ 账号的连接：QQ 掉线 / 心跳超时 → 停止全部机器人 → 非 0 退出，
  由 systemd 按 `Restart=on-failure` 重新拉起（重新拉起就等于重连，见 [deployment.md](docs/deployment.md)）。

## 二、全局硬约束（改动前必须确认不违反）

1. **Kovi 的边界：用它省事的部分，不用它自带的那套体系**。
   用：消息 / 事件 / 插件框架（`#[kovi::plugin]`、`PluginBuilder`、`kovi::plugins!`、`kovi::spawn`、
   `kovi::serde_json`、它的任务管理）。
   不用：它的 logger（`kovi::logger::try_set_logger_use_env` —— 我们自己写终端 + 文件，见
   `src/process_log.rs`）与它自己的配置（`build_bot!` 会调 `load_local_conf()` 读写 `kovi.conf.toml`、
   顺带设置 `RUST_LOG`；本项目的配置只在 `mc-qq-fishbot.conf.toml`）。
   所以入口写 `Bot::build` + `mount_plugin_set` + `kovi::plugins!` —— `build_bot!` 的全部内容就是这几步
   加上那两个"不用"的东西。
2. **尽可能避免向部署目录之外写文件**。所有生成的文件都在「部署目录」（可执行文件所在目录）或
   「机器人目录」（`paths.base_dir/bots/QQ号/机器人名`）里。需要临时目录时必须写清原因与位置。
   唯一的例外是 kovi 自己在**当前工作目录**写出的 `kovi.conf.toml` / `kovi.plugin.toml`
   （部署时 `WorkingDirectory` 就是部署目录；本项目不读也不写这两个文件，见
   [deployment.md 第九节](docs/deployment.md)）。
3. **文档先行**。先在本文件/`docs/` 里写明预期行为与对外 API，再实现；实现必须与文档逐条对上，
   不允许"文档写了、代码没做"或"代码做了、文档没写"。
4. **一个文件一件事**。文件名要能直接说明它是哪个模块的哪一类功能；一个函数不要同时干
   「建资源 + 起进程」这种两件事 —— 步骤要显式拆开（见第四节启动管道）。
5. **命名**：函数用动词短语（`start_bot`、`create_bot_log_file`），取返回值用名词且不加 `get_`
   （`latest_log_path`、`running_bot_names`），判断用 `is_`/`has_`；
   **转换用 `to_`：可能失败的转换返回 `Result`，`Err` 是可以直接回复给用户的文案**
   （`raw_bot_name.to_clean_bot_name()`、`to_owner_qq`）；构造一个类型时，已知事实用 `new`、需要去找用 `find_*`
   （`BotDirEntry::new` / `BotDirEntry::find_for_caller`）。
   变量名要带上下文与形态：**列表叫 `*_names` / `*_entries` / `*_paths`，计数叫 `*_count`**
   （不允许出现裸 `count`、`list`、`data`、`arr`）；不用自制简称。
   允许的常规写法：单位后缀 `_ms`/`_secs`、一行闭包里的 `|e|`（错误）、`bot_dir_path` 这类复合词。
6. **注释**：模块头 `//!` 说明"这个文件负责什么、为什么这么写"；函数 `///` 说明对外行为与边界；
   代码里的 `//` 只解释"为什么"，不复述代码。不放废话，也不写"待补充"。
   **风格范本见 `src/utils/bot_dirs.rs`**：模块头 1-3 行讲清"负责什么 + 关键口径"，
   函数注释直白短句、写清「做什么 / 谁调用 / 边界」，能点到具体函数名就点
   （如"供 `/start`、`/stop` 用"）；**有疑问、有更好写法就地标出来**——
   `?` 表示待定/存疑，`!` 表示该改，例如 `/// ?此条可能该合并进 [xxx]`。
   不要写成大段"为什么这么设计"的说明文。
   凡是涉及"**谁校验 / 谁负责**"的地方，必须点名**是哪一层**（路由层 / 业务层 / utils 原语），
   例如"权限在业务层校验：本函数由 [`crate::handle_all_bots_control`] 调用，它先判 `is_admin`" ——
   只写"由 X 统一校验"看不出层次，等于没说。

7. **路由层只切分，清洗在"真正用到它的那一层"做一次**：`src/command_router.rs` 把消息切成片段后
   原样透传（参数名以 `raw_` 起），谁用谁清洗 —— 机器人名走 `ToCleanBotName::to_clean_bot_name`（`BotDirEntry`
   构造条目时调用，非法名字返回 `Err` 而不是过滤），游戏内指令片段走
   `bot_control::command::join_bot_command`。**同一份文本全链路只清洗一次**，第二处不该再出现
   `trim()`。
   两条不变的：① 机器人名的口径唯一来源是 `utils::bot_name`（搬走就会有两处规则，下游只认"已清洗名字"）；
   ② 需要业务状态才能判的检查（Token 是否存在、会话在不在跑、权限/目录归属、`max_log_lines` 封顶）
   一律留在业务层。数字是路由层唯一的转换（`/log` 行数、`/ping` 端口），因为解析失败要当场回用法提示。
8. **配置只在需要它的地方读，而且只读需要的那几个字段**：逻辑重、值得单测的函数**吃参数**
   （`LaunchTiming` / `StopTiming` / `launch_command_line` + `LaunchCommandConfig` / `StatusScanner`）；
   只有一两行逻辑的便利函数直接读 `config::get().表.字段`（例如 `session_name_for` 只读
   `runtime.screen_prefix`、`BotTokenStatus::from_bot_dir` 只读 `runtime.token_file`）。
   **任何函数都不接收整份 `Config` 再自己挑字段**；配置启动读一次、之后常驻内存，
   所以便利函数读一个字段没有额外成本，而逻辑函数吃参数换来了可单测。
   详见 [docs/utils.md 第六节](docs/utils.md) 与 [docs/config.md 第七节](docs/config.md)。

## 三、必须关注的不变量（踩过坑的地方）

- **停止必须彻底**：所有停止入口只走 `utils::stop_sessions`（Ctrl+C → 等 → 强杀 → 复核），
  绝不允许留下占着游戏账号的孤儿 JVM；systemd 侧靠 `KillMode=control-group` 兜底。
- **日志两条线，磁盘上只有一份机器人日志**：
  - 机器人日志：`<机器人目录>/<runtime.logs_dir>/年-月-日-时-分-秒.log` —— 拉起会话前建好
    （`create_bot_log_file`），会话里 `grep -v -E` 过滤后 `tee -a` 追加；`/log`、`/status` 读它；
  - FishingBot 自己的日志：同层 `raw/log0.log`，一个指向 `/dev/null` 的符号链接。
    它建不出日志文件会拒绝启动，所以给它一个"能写、但什么都不存"的目标 —— 别再改成"保留一份原始日志"；
  - 本进程日志：`<部署目录>/logs/年-月-日-时-分-秒.log`（终端同时有一份），见 [docs/process-log.md](docs/process-log.md)；
  - 日志文件名的**唯一来源**是 `bot_log_files::create_timestamped_log_file`，别在别处拼时间戳文件名。
- **回复口径不许夸大**：`/start`、`/restart` 只能确认"命令已发送、会话已出现"，
  因此回复写「已发送…启动命令」；`/stop` 的停止流程当场复核，才可以回「已成功停止」。
  新增指令时按这条原则选词：**能验证到哪一步，就只说到哪一步**。
- **回复必须塞得进一条消息**：面向用户的长文本一律走 `Report`（`render()` 按
  `runtime.max_reply_chars` 从末尾按行截断并注明省略行数）；协议端超长会**直接发送失败**，
  用户那边看起来"什么都没发生"，别自己拼无界字符串直接回。
- **`keywords.filter_regex` 不能含单引号**（它会内联进 `bash -c` 里的 `grep -v -E '...'`）。
- **配置只在启动时读一次**：运行中改配置不生效，重启进程才生效（刻意不做热重载）。
- **`bot_dirs` 是目录口径的唯一来源**：`base_dir/bots/QQ号/机器人名`，权限边界在 QQ 号那一层。
- **"运行中不许动"要先查会话**：`/token`、`/bot del`、`/bot rename` 动手前都先
  `is_bot_screen_running`（改成运行中的 Token 不会生效、删运行中的目录会毁掉数据）；
  重启流程更严：缺 Token 的目标在**停止之前**就剔掉（重启是先停再起，白停一次会把还在钓鱼的账号弄下线）。
- **取哪批目录是各指令的口径**：`utils::split_bot_entries_by_running(&[BotDirEntry])`（在 `running_bots` 里）只做分组，
  目录范围由调用方自己挑函数决定：`all_bot_dir_entries()`（全量 —— `/startall`、`/restartall`、`/stopall`
  与进程收尾，缺 Token 的孤儿进程也要能被收掉）、`token_ready_bot_dir_entries()`（只看配好 Token 的 ——
  `/commandall`、`/status`）、`bot_dir_entries_of_qq(QQ)`（只看某个账号 —— `/show mybots`）；
  「目录 × 会话」这条 join 只允许在 `running_bots` 一处实现。
- **权限只有两层、每层只有一处**：单机器人指令与账号管理指令是**业务层**判 ——
  `BotDirEntry::find_for_caller` 按目录归属（普通用户只能碰自己 QQ 目录，管理员可跨目录）；
  批量指令是**路由层**判 —— `lib::handle_all_bots_control` 的 `is_admin`，
  `all_bots_control/*` 因此不接 `caller_qq`、也不自查。`command_router` 只数参数个数、不看权限。
- **不做单实例锁，靠"一台机器一份配置一个实例"**：kovi-onebot 是 WebSocket 客户端、不占端口，
  两个实例都能连上协议端且都不报错；本进程不持有权威状态（状态在 screen 会话与磁盘上），
  重复实例的症状只是"同一条命令回复两次"。加锁只能是机器级作用域，会挡住"换 base_dir 并行跑一份"。
  详见 [deployment.md 第八节](docs/deployment.md)。

## 四、启动管道（示范"步骤显式"的写法）

`utils::startup::start_bot_sessions` 里是四个连续的显式步骤，每个机器人都走一遍：

1. `create_bot_log_file(bot_dir)` —— 建这次运行的日志文件（时间戳命名，失败就不拉会话）；
2. `reset_fishingbot_logs_dir(&bot_logs_dir(bot_dir, logs_dir_name))` —— 准备 FishingBot 的日志目录
   （`raw/log0.log` → `/dev/null`，内容丢弃）；
3. `launch_bot_session(name, dir, log_path, game_logs_dir)` —— `screen -dmS` 起会话：
   `java … | grep -v -E '<filter_regex>' | tee -a '<log_path>'`；
4. 错峰等待 + 一次 `screen -list` 快照复核会话是否真的出现。

新增流程请沿用这个风格：**每一步一个函数、每步都能被单独读懂**。

## 五、构建与验证（提交前跑完）

```bash
cargo fmt --check                      # 格式
cargo test --lib                       # 库测试（Linux 上默认目标 musl 就能直接跑）
cargo test --bins                      # 插件入口测试
cargo check --all-targets              # 真实目标：x86_64-unknown-linux-musl（静态）
cargo doc --no-deps                    # 文档链接：必须零警告（只用 `cargo check` 抓不到断链）
```

* 在 Windows 主机上跑测试要显式换目标：`cargo test --lib --target x86_64-pc-windows-msvc`
  （musl 的测试二进制在 Windows 上执行不了；Linux 上直接跑默认目标即可）。
改动配置字段必须同步 `mc-qq-fishbot.conf.example.toml`，否则 `cargo test` 会失败。
文档注释里引用**私有**条目（如 `utils::*`、`settings`）时用反引号，不要写成方括号链接 ——
公开项的文档链到私有项会触发 `rustdoc::private_intra_doc_links` 警告。

## 六、代码地图（按"看名字就知道"的顺序）

```
src/main.rs              进程入口：配置 → 本进程日志 → kovi Bot → 运行 → 断开即收工
src/process_log.rs       本进程日志：终端 + <部署目录>/logs/时间戳.log
src/lib.rs               指令入口层：动作枚举 + handle_* + 进程收工入口
src/command_router.rs    kovi 插件：认指令 → 洗参数 → 分发（心跳事件的注册在这里）
src/heartbeat_event.rs   OneBot 心跳事件：认 meta_event.heartbeat 载荷
src/heartbeat.rs         心跳看门狗：判定窗口与收工判定
src/config.rs            外置配置：load/get/path/admins/install_dir
src/bot_control/         单机器人指令（一指令一文件）
src/bot_manage/          账号管理指令（一指令一文件）
src/all_bots_control/    全服批量指令（仅管理员，一指令一文件）
src/show/                只读查询指令（一指令一文件）
src/general/             通用指令（一指令一文件）
src/utils/               原语层：bot_dirs / bot_name / bot_log_files / log_status /
                         running_bots / screen / shutdown / startup / uptime /
                         report / permission
```

## 七、已经定型的两个技术选择（讨论过，别再反复改）

1. **机器人日志只留一份**：写入前过滤（`grep -v -E` → `tee -a`），FishingBot 自己的日志文件指向
   `/dev/null`。不采用"原始日志写 `/tmp` + 过滤后写机器人目录"的双份方案 —— 磁盘（或 tmpfs 内存）
   多一份、要额外的清理与权限逻辑，而且历史上正是"两份日志落在同一层目录"让 `/log` 读错文件。
   要排查过滤规则时，临时把 `keywords.filter_regex` 调空并重启进程即可看到全部行，不必常驻一份副本。
2. **模块用新版风格**（`src/utils.rs` + `src/utils/*.rs`），不用 `mod.rs`。两者对编译器完全等价，
   但新版风格在编辑器标签页与搜索结果里能直接看到模块名，且全项目零 `mod.rs`，风格统一。

## 八、当前状态与待办

- 最近一轮：按 `bot_dirs.rs` 的新批注收口 —— 条目构造改为「原始名字 + 归属 QQ 号」，
  路径由 `bots_dir/QQ号/名字` 拼出来（不再传路径）；`create_bot_dir` 成功返回建出来的条目；
  删掉 `expected_bot_dir_path`（纯路径拼串变成私有 `bot_dir_path`，只收已清洗的名字）；
  **路由层不再清洗参数**：`clean_args` 删除，`/command`、`/commandall` 把原始片段 `&[&str]` 透传，
  由 `join_bot_command` 拼一次（`/pause`、`/unpause` 也走它），`/token` 原样透传 `raw_token`；
  参数名统一成 `raw_*` / `clean_*`，同一份文本全链路只清洗一次（`/bot del` 因此只剩一次清洗，
  找不到时统一回 `BotDirEntry::find_for_caller` 的"未找到"文案）；`token_status` 收进
  `BotTokenStatus::from_bot_dir`（少一个自由函数，调用点一眼看出参数是"机器人目录"）。
- 上一轮：把"找目录"并进条目构造 —— 自由函数 `find_bot_dir_for_caller` 与 `to_bot_dir_entry`
  合并成 `BotDirEntry` 的两条构造入口：`BotDirEntry::new(名字, 已知路径)` 与
  `BotDirEntry::find_for_caller(名字, 调用者 QQ)`（内部先找到路径再走 `new`，因此只有名字也能补全
  QQ 号 / Token 状态）；磁盘扫描走私有的 `BotDirEntry::from_scanned_dir`（名字不校验，
  手工建的非法名目录仍要能出现在列表与失败明细里）；`stop_one_bot_session` 改为直接收条目。
- 上一轮：按 `bot_dirs.rs` 的批注继续收敛 —— `BotDirEntry` 从元组改成**结构体**（字段 `owner_qq` /
  `bot_name` / `bot_dir_path` / `bot_token_status`），Token 状态在构造条目时取一次，各处的
  `token_status(path)` 调用因此收进条目里；`BotTokenStatus` 保留枚举（同意）；
  `walk_all_bot_dirs` 与 `all_bot_dir_entries` **合并**（遍历 + 排序同一个函数，查找多一次排序可忽略，
  `/show allbots` 的顺序反而稳定了）；注释按"少说废话"再压一遍。
- 上一轮：机器人名净化改为 `to_clean_bot_name`（返回 `Result`，**非法字符报错不再静默过滤**），
  `to_owner_qq` / `to_bot_dir_entry` 同样带 `Result`（反推不出归属 QQ 就报错，不再记 0），
  `TokenStatus` → `BotTokenStatus`、`bot_dir_entries_of_owner` → `bot_dir_entries_of_qq`（参数 `query_qq`）、
  `split_bot_entries_by_token_ready` → `split_bot_entries_by_token_status`；会话名口径（`session_name_for`）
  随之返回 `Result`，拼不出会话名的目录按"不在运行 / 不参与复核"处理。
- 上一轮：修掉文档里已成旧名的 `find_authorized_bot_dir` / `BotDirFilter` / `bot_dir_entries(..)` 与
  `walk_all_bot_dirs_with_owner`（改为 `find_bot_dir_for_caller` / `all_bot_dir_entries()` /
  `token_ready_bot_dir_entries()`），并补齐 `docs/utils.md` 的章节编号（「七、八」→「六、七」）。

- 上一轮：私聊新增 `/help`、未识别指令统一回「错误的指令，输入/help 开始使用」；
  恢复 `kovi::spawn` 拉看门狗（长驻任务交给框架登记）；删除仓库里无用的 `old_*.rs` / `old_conf.toml`；
  FishingBot 的日志改为写向 `/dev/null`（磁盘上只剩过滤后的那一份）；`/start`、`/restart` 回复改为
  「已发送…命令」，`/stop` 保持「已成功停止」。
- 已知待办（未做，按需再做）：本进程日志不自动轮转（长期运行需要手动清理）。
