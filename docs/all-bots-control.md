# all_bots_control：全服批量指令

* 代码：`src/all_bots_control.rs` + `src/all_bots_control/*.rs`
* 入口：`lib::handle_all_bots_control`（动作枚举 `AllBotsControlAction`）—— 它同时是这批指令的**路由层**
* 权限**只在路由层校验一次**（`handle_all_bots_control` 的 `is_admin`）：本模块的函数一律不接
  `caller_qq`、也不自查，避免四份重复文案与口径不一致。本模块只做「调批量流程 + 拼报告」。

## 一、指令一览

| 指令 | 函数 | 作用范围 |
| --- | --- | --- |
| `/startall` | `startall::start_all_bots()` | 所有「已配 Token 且未运行」的机器人 |
| `/stopall` | `stopall::stop_all_bots()` | 所有运行中的机器人（**含 Token 已失效的残留进程**） |
| `/restartall` | `restartall::restart_all_bots()` | 所有「已配 Token 且正在运行」的机器人 |
| `/commandall <指令>` | `commandall::send_command_to_all_bots(bot_command)` | 所有运行中的机器人 |

这些函数都不接 `caller_qq` 参数：权限在路由层 `handle_all_bots_control` 就拦下了，能进到这里的一定是管理员。
（对照：单机器人指令的权限在各业务函数内部按目录归属判定，见 [bot-control.md](bot-control.md)。）

## 二、名单口径

取哪批目录由**各指令自己**决定（`all_bot_dir_entries()` / `token_ready_bot_dir_entries()`），再用
`utils::split_bot_entries_by_running(&[BotDirEntry])` 分组（一次 `screen -list` 快照）：

| 指令 | 目录口径 | 为什么是这批 |
| --- | --- | --- |
| `/startall` | `all_bot_dir_entries()`，先按 Token 就绪拆成两组 | 缺 Token 的账号不是"失败"，而要**看得见**（进跳过明细），所以取全部再分组 |
| `/restartall` | `all_bot_dir_entries()`，只取「在运行」的那组 | 未运行的目标重启等于启动（那是 `/startall` 的事）；缺 Token 的由重启流程在**停止之前**剔掉 |
| `/stopall`、进程退出收尾 | `all_bot_dir_entries()` | Token 被删掉之后，之前拉起的进程可能还在跑并占着游戏账号，收尾时必须能扫到它们 |
| `/commandall` | `token_ready_bot_dir_entries()`，只要名字 | 发指令不需要目录；口径与 `/status` 一致（只发给配好 Token 且在跑的） |

`/restartall` 刻意**不**按 Token 过滤：重启是"先停再起"，而缺 Token 的目标起不来（登录必然失败），
白停一次会把一个可能还在钓鱼的账号弄下线。过滤因此放在重启流程里、且放在停止之前，
那些目标会以「缺少 Token 文件，已保持原状」出现在失败明细里，而不是静默消失。

## 三、各指令的流程

* **`/startall`**：先按 Token 就绪拆成两组（`split_bot_entries_by_token_status`），
  「有效账号」再按运行状态拆开（`split_bot_entries_by_running`）。
  一个有效账号都没有 → `未配置任何有效账号，未启动。`（若还有缺 Token 的账号，会追一句提示）；
  全部在运行 → `全部有效机器人均已运行`；否则
  `utils::start_bot_sessions(&not_running_bot_entries, LaunchTiming::batch())`，
  之后 `Report::set_counts` 覆盖计数行
  （有效账号总数 / 已发送启动命令 / 跳过 (已在运行) / 跳过 (缺少 Token) / 启动失败）；
  **没被启动的账号都进 `skipped` 明细**（已在运行、缺 Token 两种原因各一条），不算失败。
* **`/stopall`**：`utils::stop_all_bot_sessions()` 一步到位（它自己就是 `all_bot_dir_entries() + split + stop` 的组合）。
* **`/restartall`**：`all_bot_dir_entries()` 里只取「在运行」的那一组（未运行的不算重启对象）；
  没有时直接回 `没有运行的机器人实例，无需重启`；否则 `utils::restart_bot_sessions`，
  再用 `Report::set_counts` 写 `运行中账号 / 已发送重启命令 / 重启失败`
  （缺 Token 的目标会在「重启失败」明细里说明原因）。
* **`/commandall`**：指令片段由插件入口原样透传，这里用 `join_bot_command` 拼成一条指令
  （与 `/command` 同一口径），再逐条注入；
  `utils::running_bot_names(&token_ready_bot_dir_entries()?)` 来自一次
  `screen -list` 快照，命中即运行中，因此这里直接逐条注入，不再逐个机器人重复鉴权与探测会话；
  逐条统计成功数，失败的原文放进 `Report::failed`。

## 四、报告约定

* 四条指令都输出 `Report`：固定「标题 + 计数行 + 明细行」，
  计数行统一由 `Report::set_counts` 生成，格式为 `标签: N 个`。
* 回复长度有上限：`Report::render()` 会按 `runtime.max_reply_chars` 从末尾按行截断并注明省略行数 ——
  账号多时批量明细最容易触到上限（协议端超长会直接发送失败）。
* **计数标签不许夸大**：启动类标签是「已发送启动命令」「已发送重启命令」，
  不是「成功启动」—— 批量启动只能确认命令发出与会话出现（与 `/start` 同一条口径，
  见 [bot-control.md 第二节](bot-control.md#二三个动作函数的固定步骤)）；
  停止类才用「成功停止」（停止流程当场复核）。
* 「基础信息要等流程跑完才知道」的指令（`/startall`、`/restartall`）先跑流程、
  再用 `Report::set_counts(..)` 覆盖计数行 —— 这是显式覆盖，不会与流程写入的行叠加。

## 五、失败排查

批量启动的瞬时内存峰值常高于云服务器可用内存，被 OOM 杀掉的进程不会出现在 `screen -list` 里，
因此结果里会写「启动指令已发送，但进程不存在（通常是内存不足）」。
处理方式：调小 `runtime.java_opts` 的内存配额、调大 `timing.batch_stagger_ms` 增加错峰，
或稍后重复一次命令（JVM 内存占用会随时间优化）。
