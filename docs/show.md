# show：状态与信息查询

* 代码：`src/show.rs` + `src/show/*.rs`
* 入口：`lib::handle_show`（动作枚举 `ShowAction::Config / MyBots / AllBots / Admins`，
  由 `ShowAction::from_name` 把 `/show` 后面的单词转成枚举，未知单词给用法提示）
* 全部是只读操作：不碰会话、不写磁盘，因此不需要「运行中」检查。

## 一、指令一览

| 指令 | 函数 | 数据来源 |
| --- | --- | --- |
| `/status` | `status::status()`（内部走 `utils::running_bots_status()`） | `screen -list` + 各机器人最新一份过滤后的日志 |
| `/show config` | `config::global_game_config()` | `paths.game_config`（所有机器人共用的那份） |
| `/show mybots` | `mybots::my_bots_list(caller_qq)` | 调用者自己 QQ 目录下的机器人 |
| `/show allbots` | `allbots::all_bots_list()` | 全部 QQ 目录下的机器人 |
| `/show admins` | `admins::admins_list()` | 配置里的 `[admins]` |

## 二、`/status`

* 名单是「已配 Token」与「会话存在」的交集：先取 `utils::token_ready_bot_dir_entries()`
  （与 `/commandall` 同一口径），再由 `utils::running_bots_status()` 用一次 `screen -list`
  快照把不在跑的剔掉。所以出现在表里的都是「配好 Token 且会话存在」的机器人；
  Token 文件被删掉的在跑进程不在表里（`/stopall` 仍能收掉它）。
* 每个机器人的状态由 `utils::running_bots` 从**最新一份日志**里判定（从末尾往回扫，
  两类状态都有结论就停；挑文件的口径见
  [utils.md 第七节](utils.md#七日志有两条线)）。措辞来自 `utils::log_status`：
  服务器状态是 `就绪/初始中`、`服务器在线`、`重连/本人在线`、`进程已结束`、`未生成日志`、
  `读取日志失败`、`日志为空`；钓鱼状态是 `钓鱼中` / `已暂停`。
* 运行时长由 `utils::format_uptime` 统一格式化成「天-时-分」，避免每个调用点各写一套。
* 输出有固定列顺序（`机器人 | 服务器状态 | 暂停状态 | 运行时长`），逐台一行，方便肉眼扫。

## 三、`/show config`

* 打印的是 `paths.game_config` 指向的**钓鱼配置原文**（FishingBot 自己的 `config.json`），
  不是本项目的 `mc-qq-fishbot.conf.toml`：聊天里要看的通常是「钓鱼参数现在是什么」。
* 文件缺失或读不出来时，回一条说明原因的错误文案，而不是静默返回空。

## 四、`/show mybots` 与 `/show allbots`

* 两者共用同一套渲染、同一份目录口径（都在 `utils::bot_dirs` 里，都按机器人名排序），
  差别只在「取哪些目录」：`my_bots_list` 走 `bot_dir_entries_of_qq(QQ)`，
  `all_bots_list` 走 `all_bot_dir_entries()`。
* 每台列出：机器人名、创建者 QQ（`allbots` 才有意义）、Token 是否存在
  —— Token 只报「在 / 不在」（`BotTokenStatus`），不报内容、也不联网校验真实性。
* 正因为不分权限地展示全部账号，`/show allbots` 的信息是「管理员视角」的；
  它本身**谁都能用**（不查权限），只是普通用户通常只看自己的目录，用 `/show mybots` 即可。
  别把别人的机器人名念出来给别人看 —— 名字本身不算敏感，但没必要。

## 五、`/show admins`

* 逐行输出 `名字 : QQ号`，名字只来自配置，不做任何联网查询。
* 名单为空时给一句明确提示（`还没配置管理员：在 mc-qq-fishbot.conf.toml 的 [admins] 里加一行 QQ号 = "名字"`），
  避免看起来像出错。
