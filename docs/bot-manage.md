# bot_manage：机器人账号管理指令

* 代码：`src/bot_manage.rs` + `src/bot_manage/*.rs`
* 入口：`lib::handle_bot_manage`（动作枚举 `BotManageAction`）
* 权限**不在路由层**，而是各子模块内部按"目录归属哪个 QQ 号"判定：
  `add` 落在调用者自己的 QQ 目录下；`del` / `rename` / `token` 由 `BotDirEntry::find_for_caller` 判定
  （管理员可跨目录）。插件入口 `command_router` 只数参数个数。
* 共同点：**目录名与后台日志一律使用规范化后的名字**（`utils::ToCleanBotName::to_clean_bot_name`，
  名字非法时它返回 `Err`、由用户改名重试），这样回复、日志、磁盘目录名三者始终对得上。

## 一、指令一览

| 指令 | 函数 | 权限 |
| --- | --- | --- |
| `/bot add <机器人名>` | `add::add_bot(raw_bot_name, caller_qq)` | 所有人（只会创建在自己 QQ 目录下） |
| `/bot del <机器人名>` | `del::delete_bot(raw_bot_name, caller_qq)` | 创建者或管理员 |
| `/bot rename <旧名> <新名>` | `rename::rename_bot(caller_qq, raw_old_bot_name, raw_new_bot_name)` | 创建者或管理员 |
| `/token <机器人名> <token>` | `token::update_bot_token_file(raw_bot_name, token_string, caller_qq)` | 创建者或管理员（仅私聊；运行中不可改） |

## 二、`/bot add`

* 名称合法性（`utils::ToCleanBotName::to_clean_bot_name`：非法字符直接报错，不再过滤）与全服重名校验都在
  `utils::create_bot_dir` 里，它成功时返回**建出来的条目**（回复与后台日志都用条目里的 `bot_name`）。
* 目录始终落在调用者自己的 QQ 目录下：路径由调用者 QQ 号推导，调用者写不到别人的目录。
* 回复里明确提醒「还需配置 Token 后才可启动」。

## 三、`/bot del`

1. `BotDirEntry::find_for_caller(..)` 定位目录；定位不到就按「不存在」回复
   —— 不暴露「该机器人在别人名下」这类信息。
2. 会话在运行时拒绝删除（`refusal_if_running`，统一文案见 `utils::running_bots`）：
   否则会删掉它正在写入的数据目录。
3. `fs::remove_dir_all` 删除目录，成功/失败都会写后台日志。

## 四、`/bot rename`

顺序是刻意安排的，不要调整：

1. 校验新名非空、且与旧名不同。
2. `BotDirEntry::find_for_caller(旧名)` 定位旧目录（口径＝目录归属，管理员可跨目录）。
3. 旧会话在运行时先 `stop_bot`，再等 `timing.rename_settle_ms` 缓冲文件句柄释放
   （专用参数，不再借用批量的 `batch_stagger_ms`）。
4. **重名检查放在停止之后**：停止失败的账号不该改名。
5. `fs::rename` 在同一个 QQ 目录里换名字（新旧目录同属一个账号）。

`/bot rename` 之后不会自动重启：需要时手动 `/start`（避免改名同时拉起带来账号争用）。

## 五、`/token`

* 只写文件、**不校验 Token 有效性**：写进去的内容由 FishingBot 自己判断。
* **运行中的账号禁止改 Token**：正在跑的 JVM 只在启动时读一次 Token 文件，改它不会生效，
  却容易让人以为"已经换好了"。要换就先 `/stop`（与 `/bot del`、`/bot rename` 同一条规矩）。
* Token 由**插件入口**原样透传（按空白切分时首尾空白已经掉了），业务函数收到的是 `raw_token`，写入前不再改写。
* 文件名为 `runtime.token_file`，写在机器人目录下；`fs::write` 不存在时创建、存在时覆盖。
* Token 文件是「能不能启动」的判据：条目里的 `bot_token_status`（`BotTokenStatus::Ready` / `Missing`），
  判据本身只在 `BotTokenStatus::from_bot_dir` 一处。
