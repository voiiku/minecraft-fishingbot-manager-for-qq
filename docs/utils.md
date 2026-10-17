# utils：公共工具层

* 代码：`src/utils.rs`（只管模块声明与再导出）+ `src/utils/*.rs`（一个主题一个文件）
* 命名约定：**动作 + 对象**（`raw_bot_name.to_clean_bot_name()`、`is_bot_name_taken`、`read_last_lines`），
  返回 `Result<_, String>` 的地方 `Err` 一律是可以直接给用户看的文案。
* 只有被上层用到的模块才在 `utils.rs` 里 `pub use`：
  `log_status` 是**内部实现细节**（只服务 `running_bots`），不对外可见。

## 一、模块地图

| 模块 | 职责 | 主要入口 |
| --- | --- | --- |
| `bot_name` | 机器人名的规范化与合法性（非法名字报错，不过滤） | `ToCleanBotName::to_clean_bot_name`（`str` 扩展 trait，`&str` / `String` 都能直接调） |
| `bot_dirs` | 目录定位、遍历、创建、Token 状态与「取哪批目录」 | `BotDirEntry`（结构体；构造入口 `BotDirEntry::new`（原始名字 + 归属 QQ 号）/ `BotDirEntry::find_for_caller`（原始名字 + 调用者））/ `all_bot_dir_entries` / `token_ready_bot_dir_entries` / `bot_dir_entries_of_qq` / `is_bot_name_taken` / `split_bot_entries_by_token_status` / `create_bot_dir`（成功返回条目）/ `BotTokenStatus`（判据 `BotTokenStatus::from_bot_dir`）|
| `screen` | screen 会话：快照、探测与指令注入（注入是不做校验的原语） | `active_screen_sessions` / `is_screen_session_running` / `is_bot_screen_running` / `session_name` / `session_name_for` / `send_command_to_single_bot` |
| `running_bots` | **「谁在跑」的唯一口径**：状态汇总、按运行状态分组、运行中的名字、运行中拒绝文案 | `running_bots_status` / `RunningBotInfo` / `split_bot_entries_by_running` / `running_bot_names` / `refusal_if_running` |
| `startup` | 启动与重启（四步显式流程；缺 Token 的目标不拉起） | `start_bot_sessions` / `restart_bot_sessions` / `LaunchTiming` / `launch_command_line` + `LaunchCommandConfig`（`launch_bot_session` 是内部私有步骤，不对外） |
| `shutdown` | 停止会话 | `stop_one_bot_session` / `stop_all_bot_sessions` / `StopTiming` |
| `bot_log_files` | 机器人日志：创建、挑选、读取；FishingBot 的日志指向 `/dev/null` | `create_bot_log_file` / `create_timestamped_log_file` / `bot_logs_dir` / `reset_fishingbot_logs_dir` / `latest_log_path` / `read_log_lines_backwards_until` / `read_last_lines` |
| `log_status` | 从日志逐行判定状态（内部；`StatusScanner`） | `StatusScanner` / `ConnectionStatus` / `PauseStatus` |
| `uptime` | 运行时长：读进程已运行秒数 + 格式化 | `process_elapsed_seconds` / `format_uptime` |
| `report` | 统一的回执构造（带长度上限） | `Report::set_counts` / `absorb` / `is_ok` / `render` + `IntoReplyText` |
| `permission` | 权限判定 | `is_admin` |

## 二、`bot_dirs`：Token 状态与「取哪批目录」

* `BotTokenStatus::Ready / Missing`：只判断 Token 文件「有内容 / 没内容」，
  不联网校验有效性 —— 有效性只有 FishingBot 自己启动时才知道。
* `BotDirEntry` 是目录条目的结构体：归属 QQ 号 + 机器人名 + 文件夹路径 + 构造时取到的 Token 状态。
  路径不用传（由 `bots_dir/QQ号/名字` 拼出来），两条构造入口表达"归属从哪来"：
  `BotDirEntry::new(原始名字, 归属 QQ 号)` 与 `BotDirEntry::find_for_caller(原始名字, 调用者 QQ)`
  （在权限范围内找到归属再建条目）。名字都在构造里清洗一次。
* 「取哪批目录」不做成过滤枚举，而是**一个口径一个函数**（名字直接说明范围，调用处不必再查枚举含义）：
  * `all_bot_dir_entries()` —— 全部 QQ 目录（`/startall`、`/restartall`、`/stopall`、进程收尾、`/show allbots`）；
  * `token_ready_bot_dir_entries()` —— 只看配好 Token 的（`/commandall`、`/status`）；
  * `bot_dir_entries_of_qq(QQ)` —— 只看某个 QQ 号名下的（`/show mybots`）。
  上层不再自己拼 `read_dir` 循环，避免不同模块过滤条件写得不一致。

## 三、`running_bots`：「谁在跑」的唯一口径

`screen -list` 只调用一次并缓存成快照，之后所有「在不在运行」的判断都基于同一份快照。
**目录 × 会话的连接（join）只在本模块做**，别处不要再遍历目录去匹配会话名：

* `running_bots_status()` → `Vec<RunningBotInfo>`（机器人名 + 连接/暂停状态 + 运行时长；`/status` 用）
* `split_bot_entries_by_running(&[BotDirEntry])` → `(在运行, 未运行)`；**取哪批目录由调用方决定**
  （`all_bot_dir_entries()` / `token_ready_bot_dir_entries()`），见 [all-bots-control.md 第二节](all-bots-control.md#二名单口径)
* `running_bot_names(&[BotDirEntry])` → 只要名字（`/commandall` 用），内部就是上面那个的「在运行」那一半

「一次快照、多处复用」是有意的：批量流程里若每次都重新探测，
前面刚启动的会话会立刻被后面算成「已在运行」，统计口径就会漂。

## 四、`startup` / `shutdown` 的等待参数

* 拉起会话的等待参数收成一个具名参数 [`LaunchTiming`]：
  * `LaunchTiming::single()` —— 单个机器人指令（不错峰，等 `timing.single_settle_ms`）；
  * `LaunchTiming::batch()` —— 批量指令（错峰 `batch_stagger_ms`，等 `batch_settle_ms`）。
  调用处因此不会出现 `start_bot_sessions(&entries, 0, 1500)` 这种要查文档才知道的裸数字。
* 批量的总耗时 ≈ `(台数 - 1) × batch_stagger_ms + batch_settle_ms`：**最后一台不再空等**错峰
  （后面本来就有 settle 等待）。账号很多时调小 `batch_stagger_ms` 是唯一的提速手段，
  代价是 JVM 启动的内存峰值变高（见 [all-bots-control.md 第五节](all-bots-control.md)）。
* 停止的等待参数同样是具名的 [`StopTiming`]（`StopTiming::from_config()` 只读
  `batch_stop_grace_ms` 与 `batch_stop_settle_ms` 两个字段）；停止流程在 `stop_sessions` 内部读它，
  函数自己不再碰配置。
* 停止流程先发退出信号，等 `batch_stop_grace_ms` 再不成就强杀，
  强杀后再等 `batch_stop_settle_ms` 让旧会话彻底注销 —— 立刻复核会读到还没消失的旧会话。
* 复核统一走 `running_bots`，所以「拉起成功」的定义是**会话真的出现**（一次 `screen -list` 快照），
  不是「命令已发出」；但会话出现也**不等于已经上线**（登录、进服由 JVM 自己跑），
  报告与回复因此只写「启动命令已发送，会话已出现」。

## 五、`report`：回执格式只写一处

* 字段即小节：`basics`（基础信息与列表行，**列表行直接 push**）、`succeeded` / `skipped` / `failed`
  （明细）、`content`（整块内容）；`render()` 会自动省略没填的部分，所以空节不会留下空标题。
* `set_counts(&[(标签, 数量)])`：**覆盖**基础信息行，统一生成 `标签: N 个`；
  非计数行（表头、名单、说明）直接往 `basics` 里 push。
* `untitled()` + `set_title()`：标题要等流程跑完才能确定的场景；`absorb(..)` 合并「停止 + 启动」
  这类组合流程的报告（明细相加、内容取第一个非空的）；
* `is_ok()` / `failure_text()`：单个机器人指令判断成败、取一行错误提示。
* `render()` 会按 `runtime.max_reply_chars` **收口长度**：超长时从末尾按行丢，
  并在结尾注明省略了多少行（协议端对单条消息有长度限制，超长会直接发送失败，
  用户那边看起来"什么都没发生"；按行截断则保证标题与计数行一定还在）。

计数要等流程跑完才知道的指令，先跑流程、再用 `set_counts` 整体覆盖，
而不是让流程往里追加 —— 否则会同时出现占位信息与真实结果两套数字。

## 六、配置取值约定

`utils` 里**没有**"接收整份配置再自己挑字段"的函数 —— 每个函数只碰它真正用到的那一两个字段，
分两类写法：

| 写法 | 用在 | 例子 |
| --- | --- | --- |
| **吃参数**：值由调用方给，函数不碰配置 | 逻辑重、值得单测的地方 | `LaunchTiming::{single,batch}`、`StopTiming`（成对的两个毫秒数）、`launch_command_line` + `LaunchCommandConfig`（启动命令拼接）、`StatusScanner`（状态判定）、`Report` 的长度截断、`bot_logs_dir`、`session_name`（纯拼串/拼路径） |
| **只读它需要的字段**：`config::get().表.字段` | 只有一两行逻辑的便利函数 | `session_name_for` → `runtime.screen_prefix`；`create_bot_log_file` / `latest_log_path` → `runtime.logs_dir`；`BotTokenStatus::from_bot_dir` → `runtime.token_file`；`BotDirEntry::find_for_caller` → `paths.bots_dir` + `[admins]`；`Report::render` → `runtime.max_reply_chars` |

配置"启动读一次、之后常驻内存"（见 [config.md 第一节](config.md)），所以便利函数读某一个字段
没有额外成本；而**逻辑函数吃参数**换来的是：启动命令、状态判定、等待时间这些容易出错的地方
都有单测（见 `utils::startup` / `utils::log_status` / `utils::report` 的测试）。

## 七、日志有两条线

### 1. 机器人日志：只有一份

`<机器人目录>/<runtime.logs_dir>/` 里就这两样东西：

| 文件 | 是什么 |
| --- | --- |
| `年-月-日-时-分-秒.log` | **我们的日志**：拉起会话前由 `create_bot_log_file` 建好，会话把 JVM 输出经 `grep -v -E` 过滤后 `tee -a` 追加（命中 `keywords.filter_regex` 的行被丢掉）。`/log` 读它的尾部，`/status` 从**末尾往回扫**它判状态 |
| `raw/log0.log` | **FishingBot 的日志**：一个指向 `/dev/null` 的符号链接，写进去的内容由内核丢弃 |

`/status` 为什么要倒着扫（`read_log_lines_backwards_until` + `log_status::StatusScanner`）：
状态判定只看"最后一次出现的关键词"，所以从最新一行往回读、两类状态都有结论就停 ——
长跑账号的日志可能有几十 MB，整份读进来既慢又占内存。

为什么还给 FishingBot 留个文件：它建不出日志文件会**直接拒绝启动**
（`Bot.java`：`FileHandler` 抛 `IOException` → `setRunning(false)`），所以只能给它一个"能写、
但什么都不存"的目标 —— 磁盘上因此只有一份真正的日志，也不会再有两份互相干扰。

挑文件的口径只在 `bot_log_files::latest_log_path` 一处，且只认时间戳命名（由 `create_timestamped_log_file` 生成）。
这是防退化的保险：早先按"最后修改时间"挑 `.log`，实时刷盘的 `log0.log` 更"新"，
`/log` 于是读回了未过滤的刷屏行。

### 2. 本进程日志：终端与文件各一份

`src/process_log.rs` 装的 logger（`init_process_log`，进程入口在装配 kovi 之前调用）
把 kovi、kovi-onebot 与库里的 `log::*` 同时写两处：

| 输出 | 位置 | 用途 |
| --- | --- | --- |
| 终端（stderr） | `journalctl -u mc-qq-fishbot` / 前台输出 | 实时看；systemd 托管时也能追溯 |
| 文件 | `<部署目录>/logs/年-月-日-时-分-秒.log` | 事后翻查（journal 会轮转） |

* 命名与机器人日志同一套（都出自 `create_timestamped_log_file`），每条记录形如
  `[2026-09-28 15:30:12] [INFO] …`，纯文本、不带颜色，所以文件干净；
* 级别：`RUST_LOG` 优先（**只认单个级别词**，如 `debug`），未设置时按配置的 `[kovi] debug`
  —— kovi 的 env_logger 支持 `模块=级别` 这类 directive，本项目刻意只保留「一个级别」的口径；
* 本项目自己的诊断信息也走 `log`（不再用 `println!`），所以终端与日志文件是同一份完整内容；
* 日志文件建不出来（部署目录不可写）只在终端留一条告警，不会让机器人跑不起来。
