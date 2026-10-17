# lib：指令入口层（路由层）

* 代码：`src/lib.rs`
* 定位：**把「聊天指令」翻译成「动作函数」的那一层**（架构文档里叫「指令入口层」，简称「路由层」，
  见 [architecture.md](architecture.md) 第一节的术语说明），并统一处理「错误文案、回复格式」。
* 上层是 `command_router`（解析文本、数参数个数），下层是 `bot_control` / `bot_manage` / `show` / `all_bots_control`（干活）。
* **权限不在这一层统一查**：批量指令的管理员权限在 `handle_all_bots_control` 里判一次（见第三节）；
  单机器人指令与账号管理指令的权限由各业务函数内部按目录归属判定。
* **参数也不在这一层清洗**：插件入口只切分、原样透传（动作枚举里带着 `raw_*`），
  谁真正要用就由谁清洗一次（机器人名在 `utils::ToCleanBotName::to_clean_bot_name`，指令片段在
  `bot_control::command::join_bot_command`）。

## 一、命名约定

| 前缀 / 后缀 | 含义 |
| --- | --- |
| `handle_*` | **入口**：接指令、分发动作、把错误转成给用户看的文案（只有批量指令的权限在这里查） |
| `bot_*_reply` | 把一个动作枚举 + 原始参数收敛成回执文本的中间层（定义在插件入口 `command_router`，不在本层） |
| `*Action` 枚举 | 指令的「动作」维度；解析产物（**参数由插件入口原样透传**），只在这里被解释 |

四个动作枚举与对应入口：

| 枚举 | 入口 | 覆盖指令 |
| --- | --- | --- |
| `BotControlAction` | `handle_bot_control` | `/start` `/stop` `/restart` `/pause` `/unpause` `/command` |
| `BotManageAction` | `handle_bot_manage` | `/bot add` `/bot del` `/bot rename` |
| `ShowAction` | `handle_show` | `/show config|mybots|allbots|admins` |
| `AllBotsControlAction` | `handle_all_bots_control` | `/startall` `/stopall` `/restartall` `/commandall` |

## 二、两种错误口径

* **权限不足**有两种来源，文案都由产生它的那一层给：
  * 批量指令：路由层 `handle_all_bots_control` 直接返回一句
    `QQ号 X 不是管理员，无权执行此操作！`（不进入业务函数，也不经 `Report`）；
  * 单机器人 / 账号管理指令：业务函数返回 `Err`，
    文案来自 `BotDirEntry::find_for_caller` —— `未找到机器人 'X' 对应的目录，或你没有权限访问`
    （刻意不区分"不存在"与"没权限"，见 [bot-manage.md](bot-manage.md)）。
* **参数/业务错误**：业务函数返回 `Result<String, String>`，`Err` 里是**给用户看的文案**；
  入口层只负责把它当回复返回，不改写措辞（避免同一件事出现两种说法）。

## 三、权限规则

* 普通用户：只能操作自己 QQ 目录下的机器人（`utils::BotDirEntry::find_for_caller` 会校验）。
* 管理员（`[admins]` 里的 QQ）：可跨目录操作任意机器人。
* 全服批量指令**只允许管理员**：在**路由层** `handle_all_bots_control` 里检查一次，
  因此 `all_bots_control/*` 的函数一律不带 `caller_qq` 参数。
* 只读查询（`/show ...`）与通用指令（`/help`、`/ping`、`/checkserver`、`/status`）一律不查权限。

## 四、进程级生命周期

除了指令处理，`lib.rs` 还导出进程收尾用的两个函数：

| 函数 | 用途 |
| --- | --- |
| `shutdown_all_bots()` | 停止所有运行中的机器人会话（不退出进程），供**正常退出前**调用（`main.rs` 的 `ExitEvent::FromSignal`） |
| `stop_all_bots_and_exit(reason)` | 停止全部机器人 + 以**非 0 退出码**退出进程（`reason` 会写进本进程日志）；心跳看门狗与连接断开都用它，交由 systemd 重新拉起 |

## 五、相关文档

* 指令怎么被解析出来：[command-router.md](command-router.md)
* 各模块细节：[bot-control.md](bot-control.md)、[bot-manage.md](bot-manage.md)、
  [all-bots-control.md](all-bots-control.md)、[show.md](show.md)、[general.md](general.md)
