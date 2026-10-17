# command_router：指令解析与分发

* 代码：`src/command_router.rs`（`main` 里的 `mod command_router;`，属于二进制 crate，不属于库）
* 定位：**只做「认指令 → 洗参数 → 分发」**，业务动作全在 `mc_qq_fishbot` 库里，便于单独阅读与测试。
* 作为 kovi 插件被挂载：`main.rs` 用官方 API `Bot::build(..)` + `mount_plugin_set(kovi::plugins!(command_router))`。

## 一、两条消息入口

| 入口 | 注册方式 | 行为 |
| --- | --- | --- |
| 群聊 | `PluginBuilder::on_group_msg` | 认出指令就回复；**未识别一律静默忽略**（群里不能对每条消息都回话） |
| 私聊 | `PluginBuilder::on_private_msg` | **以 `/` 开头的消息一律回复**（不是指令的消息静默忽略）：只放行 `/help`、`/token`、`/log`，其余给对应提示（见第五节） |

两者共用同一个解析函数，因此「怎么算一条指令」在两边的口径完全一致。

## 二、`parse_command` 的规则

```rust
fn parse_command(text: &str) -> Option<(&str, Vec<&str>)>
```

1. 按**空白**切分（`split_whitespace`，连续空格/制表符都会被合并）；
2. 第一个片段是指令本身，必须**以 `/` 开头**，否则返回 `None`（普通聊天不处理）；
3. 其余片段按原样交给各指令自己解释 —— 例如 `/command` 会把 `args[1..]` 用空格重新拼回去，
   所以「指令内容里带空格」是保留的。

注意：解析不认 `@机器人` 前缀，也不需要前缀：只要消息以 `/xxx` 开头就会被当成指令。

## 三、参数怎么往下传（只切分，不清洗）

**用户文本不在这里清洗**：本文件只把消息按空白切成片段、原样透传，谁真正要用就由谁清洗一次。
参数名以 `raw_` / `clean_` 标明形态，所以不存在"到底收到的是原始还是清洗过的"这种疑问。

| 参数 | 在这里怎么处理 | 往下传的形态 |
| --- | --- | --- |
| 机器人名 | 原样透传（顺带数个数） | `&str`（原始文本）；清洗在 `ToCleanBotName::to_clean_bot_name`（`BotDirEntry` 里调用）做一次 |
| 指令原文（`/command`、`/commandall`） | 只切分，不改写 | `&[&str]`（原始片段）；由 `bot_control::command::join_bot_command` 拼成一条指令（单空格连接、空则拒绝） |
| Token（`/token`） | 原样透传（按空白切分时首尾空白已经掉了） | `String`；写入前不再改写 |
| 日志行数（`/log`） | `parse::<usize>()`，解析失败直接回用法提示 | `usize` |
| 子命令名（`/show`、`/bot`） | `ShowAction::from_name(..)` / 匹配 `add|del|rename` | 动作枚举 |
| 端口（`/ping`） | `parse::<u16>()`，失败退回默认端口 | `Option<u16>` |

为什么要这样分：清洗规则只该有一处来源（机器人名要跟磁盘目录名严格一致，指令文本长什么样由发指令的那一层决定），
而这里做的是**切分** —— 它是解析的直接产物，不该替业务决定"参数最终长什么样"。
数字是本文件唯一的转换：解析失败要当场回用法提示，所以行数与端口在这里解析。

## 四、群聊分发

| 指令 | 参数要求 | 交给谁 |
| --- | --- | --- |
| `/help` | 无 | `handle_help()` |
| `/status` | 无 | `handle_status()` |
| `/checkserver` | 无 | `handle_checkserver().await` |
| `/ping` | `[地址] [端口]`，最多 2 个 | `handle_ping(host, port).await` |
| `/start` `/stop` `/restart` `/pause` `/unpause` | 1 个（机器人名） | `handle_bot_control(BotControlAction::.., ..)` |
| `/command` | ≥ 2 个（机器人名 + 指令内容） | `handle_bot_control(BotControlAction::Command(..), ..)` |
| `/bot` | `add|del|rename` + 名字 | `handle_bot_manage_args(..)` → `handle_bot_manage(..)` |
| `/show` | 恰好 1 个：`config|mybots|allbots|admins` | `handle_show(ShowAction::from_name(..), ..)` |
| `/startall` `/stopall` `/restartall` | 无 | `handle_all_bots_control(..)` |
| `/commandall` | ≥ 1 个（指令内容） | `handle_all_bots_control(AllBotsControlAction::CommandAll(..), ..)` |

* 参数个数不对时统一走 `usage(..)`，输出形如 `用法错误：/log <机器人名> <日志行数>`：
  **用法提示只写一处**，新增/改动指令不会漏改某一分支。
* 逐条分支都直接调 `handle_*`：**插件入口不查权限、不做业务判断**，只认指令、洗参数、数参数个数。
  权限分两层 —— 批量指令在库的路由层 `handle_all_bots_control` 查（`is_admin`），
  单机器人指令在业务层按目录归属查（`BotDirEntry::find_for_caller`），详见 [lib.md](lib.md)。

## 五、私聊分发

| 指令 | 参数 | 说明 |
| --- | --- | --- |
| `/help` | 无 | 与群聊同一份帮助文本（配置里的 `text.help`） |
| `/token <机器人名> <token>` | 恰好 2 个 | `handle_bot_manage(BotManageAction::Token(..), ..)` |
| `/log <机器人名> <行数>` | 恰好 2 个，行数必须是数字 | 解析失败就回用法提示（不会 panic） |

私聊收到其它指令时分两种口径（见 `handle_private_command`）：

* **能识别、但只在群聊提供**（`/start`、`/show`、`/startall` 等）→
  `该指令只能在群聊中使用；私聊可用：/help、/token、/log。`；
* **完全不认识**（例如 `/nonsense`）→ `错误的指令，输入/help 开始使用`。

只有 `/token`、`/log` 是私聊专属：Token 与日志属于敏感内容，群里发会留痕。
群聊收到不认识的 `/xxx` 一律**不回复** —— 群里机器人不该为打字错误刷屏。

## 六、心跳

`main` 里除了消息处理，还注册了心跳事件（类型在 [`crate::heartbeat_event`]）并拉起库里的看门狗：

```rust
PluginBuilder::on::<HeartbeatEvent, _>(|event| async move { note_heartbeat(event.report); });
kovi::spawn(watch_heartbeat());   // 唯一的长驻后台任务
```

* 长驻任务用 `kovi::spawn` 交给框架登记（插件停用时能一起收掉），不用裸 `tokio::spawn`。

* `HeartbeatEvent`（在 `src/heartbeat_event.rs`）自己实现 `kovi::event::Event`：
  kovi-onebot 没有导出心跳事件类型，因此在 `Event::de` 里按 `post_type == "meta_event"`
  且 `meta_event_type == "heartbeat"` 认载荷。
* 非心跳载荷返回 `None` 交给框架忽略；`status.online` 缺字段或类型不符按「没上报」处理。
* 心跳看门狗**不向协议端发 API 请求**（不占配额），细节见 [heartbeat.md](heartbeat.md)。
* 插件入口一个进程只会被调用一次（连接一断就收工退出、由 systemd 重新拉起），
  因此这里不需要幂等守卫。
