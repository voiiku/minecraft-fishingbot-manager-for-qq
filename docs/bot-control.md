# bot_control：单机器人控制指令

* 代码：`src/bot_control.rs`（模块文档）+ `src/bot_control/*.rs`（一条指令一个文件）
* 入口：`lib::handle_bot_control`（动作枚举 `BotControlAction`）
* 会话操作全部复用 `utils` 里的原语，本层只做「校验 + 结果转换 + 回复」
* 权限**在每个子模块内部校验**（不在路由层）：`BotDirEntry::find_for_caller` 按"目录挂在哪个 QQ 号下"
  判定 —— 普通用户只能操作自己 QQ 目录里的机器人，管理员可跨目录。
  （批量的 `/startall` 那批相反：权限在路由层 `handle_all_bots_control` 判一次。）

## 一、指令一览

| 指令 | 函数 | 说明 |
| --- | --- | --- |
| `/start <机器人名>` | `start::start_bot(raw_bot_name, caller_qq)` | 拉起会话并复核会话是否出现；**回复「已发送启动命令」** |
| `/stop <机器人名>` | `stop::stop_bot(raw_bot_name, caller_qq)` | 停止会话（释放游戏账号）；停止当场复核，**回复「已成功停止」** |
| `/restart <机器人名>` | `restart::restart_bot(raw_bot_name, caller_qq)` | 停止 + 拉起；**回复「已发送重启命令」** |
| `/pause <机器人名>` | `pause::pause_fishing(raw_bot_name, caller_qq)` | 游戏内 `/pause`，不下线 |
| `/unpause <机器人名>` | `unpause::unpause_fishing(raw_bot_name, caller_qq)` | 游戏内 `/unpause` |
| `/command <机器人名> <指令>` | `command::send_command_to_bot(...)` | 转发游戏内指令 |
| `/log <机器人名> <行数>` | `log::read_bot_log(caller_qq, raw_bot_name, line_count)` | 仅私聊 |

`/pause` 与 `/unpause` 不是独立实现：它们只是把 `/pause`、`/unpause` 当作游戏内指令交给
`send_command_to_bot`，因此**校验、权限、运行状态检查与被包裹的指令完全一致**。

## 二、三个动作函数的固定步骤

`start_bot` / `stop_bot` / `restart_bot` 的结构一致，读代码时可以按这四步看：

1. `BotDirEntry::find_for_caller(raw_bot_name, caller_qq)?`
   —— 一次调用同时保证「目录存在」与「调用者有权限」（口径＝目录归属，管理员可跨 QQ 目录），
   拿到的条目里就带着归属 QQ 号、机器人名（清洗后）、路径与 Token 状态。
2. 针对动作的**前置检查**：
   * `/start`：条目里的 `bot_token_status == BotTokenStatus::Missing` 直接拒绝（没有 Token 必然登录失败）；
     已在运行时拒绝（重复拉起会顶掉旧会话）。
   * `/stop`：会话不在运行时直接回复「无需停止」，不必走停止流程。
   * `/restart`：本函数不做前置检查（未运行等价于直接启动）；缺 Token 的目标由重启流程
     [`restart_bot_sessions`] 在**停止之前**剔掉并写进失败明细 —— 重启是"先停再起"，
     缺 Token 起不来，白停一次会把还在钓鱼的账号弄下线。
3. 调 `utils` 的流程函数（等待参数用 [`LaunchTiming`] 表达，不再传裸数字）：
   * `/start` → `start_bot_sessions(&[(name, dir)], LaunchTiming::single())`
   * `/stop` → `stop_one_bot_session(name, &dir)`
   * `/restart` → `restart_bot_sessions(&[(name, dir)], LaunchTiming::single())`
   单个指令用 `LaunchTiming::single()`（不错峰，等 `timing.single_settle_ms`）；
   批量指令才用 `LaunchTiming::batch()`（错峰 + `batch_settle_ms`），见 [utils.md 第四节](utils.md)。
4. `report.is_ok()` 为假时把 `report.failure_text()` 当错误返回，否则回一行成功文案；
   文案按**能验证到哪一步就只说到哪一步**的原则选词：
   * `/start` → `已发送机器人 'X' 的启动命令（会话已出现；是否真正上线请用 /status 确认）`
     —— 能立刻确认的只有"命令发出 + 会话出现"，登录与进服还要看日志关键词；
   * `/stop` → `已成功停止机器人 'X'` —— 停止流程当场复核会话已消失（见 `utils::stop_sessions`），
     所以这里说得起"成功"；
   * `/restart` → `已发送机器人 'X' 的重启命令（停止已确认；启动是否上线请用 /status 确认）`
     —— 停止阶段可确认，拉起阶段与 `/start` 同样只能说"已发送"。

## 三、`/command` 的细节

* 指令片段由**插件入口**切分后原样传下来（见 [command-router.md 第三节](command-router.md)），
  本函数用 `join_bot_command` 拼成一条指令（单空格连接，空则回 `发送的指令不能为空！`），只拼一次。
* 回显用的就是这条拼好的指令。
* `BotDirEntry::find_for_caller(..)?` 只取「有权访问」这个结论（口径＝目录归属），返回的条目用不上（Token 检查在 `/start` 里）。
* 会话不在运行时拒绝 —— 指令不能塞进不存在的会话。
* 注入由 `utils::send_command_to_single_bot` 完成（**screen 原语，自己不校验**；末尾自动补换行，screen 才会提交这一行）。

## 四、`/log` 的细节

* 行数必须 > 0，且被 `runtime.max_log_lines` 封顶（一条指令不能把整份大日志拉回来）。
* 只读**最新**那一份日志（`utils::bot_log_files::latest_log_path`，只认时间戳命名）：
  每个会话启动前 `create_bot_log_file` 先建好文件，会话里 `grep -v -E` 过滤后 `tee -a` 追加。
* FishingBot 自己的日志写在同层 `raw/log0.log`（指向 `/dev/null` 的符号链接，内容丢弃），
  所以磁盘上只有一份日志，`/log` 不可能读回未过滤的刷屏内容；
  细节见 [utils 的日志一节](utils.md#七日志有两条线)。
* 读取用 `utils::read_last_lines` 的滑动窗口，日志再大也只保留最后 N 行。
* 日志原文放进 `Report::content`（整块内容），避免每行被加上列表前缀；标题里带上日志文件名。
* 只有私聊能触发（分发规则见 [command-router.md](command-router.md)）。
