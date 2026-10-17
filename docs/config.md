# config：外置配置

* 代码：`src/config.rs` + `src/config/{schema,parse,validate,template}.rs`
* 文件：**可执行文件同级目录**下的 `mc-qq-fishbot.conf.toml`（文件名由 `CONFIG_FILE_NAME` 固定，不可改名）
* 刻意**不看工作目录、不读环境变量**：部署时把二进制与配置放在一起就不会错，
  systemd 之类的托管也不需要额外设置。

## 一、加载流程（启动最早期）

1. `config::load()` 定位配置文件路径；
2. 文件不存在 → 按仓库里的 `mc-qq-fishbot.conf.example.toml` 生成一份，返回
   `LoadOutcome::GeneratedTemplate`，提示使用者修改后退出，**不进 kovi**；
3. 文件存在 → 读文本 → `parse`（TOML 反序列化 + 相对路径展开）→ `validate`（取值校验）；
4. 任何一步失败都直接以「配置项 + 字段名 + 要求」的形式报错退出，不带着错值跑起来；
5. 通过后写入 `OnceLock`，**只在启动时读一次并常驻内存**：
   运行中改配置文件不影响已经在跑的进程，想让新配置生效就重启进程。

## 二、对外接口

| 接口 | 用途 |
| --- | --- |
| `load()` | 加载 + 校验；返回 `Loaded(&Config)` 或 `GeneratedTemplate` |
| `get()` | 取已加载的 `Config` |
| `path()` | 取本次启动使用的配置文件路径 |
| `admins()` | 取管理员名单（`get().admins` 的快捷方式） |
| `install_dir()` | 取**部署目录**（可执行文件所在目录）：配置、kovi 状态文件、本进程日志都在这里 |

`get()` / `path()` 在「尚未加载」时 panic：进程入口第一件事就是 `load()`，
走到这里还 panic 说明调用顺序写错了。路径与内容放在同一个 `Settings` 里，
所以「是否已加载」只有一个判断依据。

## 三、表结构

| 表 | 关键字段 | 用途 |
| --- | --- | --- |
| `[ping]` | `default_ping_host`、`default_ping_port`、`timeout_ms` | `/ping` 不带参数时的默认目标与超时 |
| `[onebot]` | `host`、`port`、`access_token`、`secure`、`path`、`all_in_one` | 组装 kovi 的 OneBot 驱动 |
| `[admins]` | `"QQ号" = "名字"` | 管理员名单；**本项目不读 kovi 自己那份配置** |
| `[kovi]` | `debug` | 日志级别（`RUST_LOG` 未设置时用它决定 info/debug；`RUST_LOG` 只认单个级别词，见 [process-log.md](process-log.md)） |
| `[keywords]` | `online`、`reconnect_prefix` / `reconnect_suffix`、`stopped`、`pause`、`unpause`、`filter_regex` | 从 FishingBot 日志判定状态；`filter_regex` 还负责在写盘前丢掉刷屏行（`grep -v -E`） |
| `[paths]` | `base_dir`（绝对）、`jar`、`game_config`（相对） | 数据根目录与共享文件 |
| `[runtime]` | `java_opts`、`screen_prefix`、`token_file`、`logs_dir`、`max_log_lines`、`max_reply_chars` | 单个机器人的运行时参数；`logs_dir` 里只有我们的日志（FishingBot 自己的日志是同层 `raw/log0.log`，指向 `/dev/null`，见 [utils.md](utils.md#七日志有两条线)）；`max_reply_chars` 是单条回复的长度上限（超长会发送失败） |
| `[timing]` | `single_settle_ms`、`batch_stagger_ms`、`batch_settle_ms`、`batch_stop_grace_ms`、`batch_stop_settle_ms`、`rename_settle_ms`、`heartbeat_timeout_ms` | 各流程等待时间（毫秒）；`rename_settle_ms` 只服务 `/bot rename`（等旧会话释放文件句柄），与批量错峰无关 |
| `[text]` | `help` | `/help` 的完整输出（多行字符串，改文案不用重新编译） |

## 四、字段规则

* **默认全部必需**：少写一个就在解析时报错；`deny_unknown_fields` 让拼错的字段名也立刻暴露。
  唯一的例外是 `timing.heartbeat_timeout_ms`（默认 `DEFAULT_HEARTBEAT_TIMEOUT_MS` = 30000），
  给它默认值是为了让**旧配置文件不改一行**也能直接升级运行。
* **路径规则**（`paths`）：`base_dir` 必须绝对路径；`jar` 与 `game_config` 必须相对路径、
  相对 `base_dir` 解析；`bots_dir` 由代码派生为 `base_dir/bots`，不写在配置里。
  相对路径的 `base_dir`、绝对路径的 `jar` 都会在启动时报错。
* `[admins]` 允许为空（那就没有管理员，各人管各人的机器人）；但键必须是纯数字 QQ 号、
  名字不能空白 —— 这两项写错会让权限**静默失效**，所以直接拦下来。

## 五、校验（`validate`）

只看「写错了就跑不起来」的三类问题（见 `validate.rs` 的模块文档）：

* **必填项不能为空**：`ping.default_ping_host`、`onebot.host`、`keywords.*`、`text.help`、
  `runtime.screen_prefix` / `token_file` / `logs_dir`（这几个还要求不含空格与斜杠、不能写成 `.` / `..`）；
* **数值范围**：`ping.default_ping_port` / `onebot.port` 不能是 0；
  `ping.timeout_ms`、`runtime.max_log_lines`、`runtime.max_reply_chars`、
  `timing.heartbeat_timeout_ms` 必须大于 0（心跳窗口为 0 会让看门狗反复判掉线）；
* **会拼进命令/路径的字符串**：`runtime.java_opts` 禁止 `' " ` $ ; & | < >` 与换行（未加引号内联）；
  `keywords.filter_regex` 不能含单引号（内联进 `grep -v -E '...'`）；
  `paths.jar` / `paths.game_config` 也不能含单引号。

另外两条属于"环境就绪"检查，同样在启动时报错：`paths.base_dir` 必须是目录、`paths.jar` 必须是文件
（写错了会一直等到拉起会话才失败，报错也远不如这里清楚）。

除此之外**一律不管**：时间参数该设多大、地址写哪个、名字里能不能用中文，都是使用者的判断。
`[kovi]` 没有规则（它只影响日志级别），`[timing]` 只有心跳窗口那一条。

## 六、模板（`template`）

* 模板就是仓库根目录的 `mc-qq-fishbot.conf.example.toml`（用 `include_str!` 内嵌进二进制，
  因此发布时不需要额外带文件）。
* 生成的模板与示例文件逐字一致；单元测试会核对示例里出现的每张表/每个字段
  （缺少字段会让照抄模板的用户直接启动失败）。

## 七、为什么是分表结构（而不是一个扁平结构体）

代码里是「一个 `Config` + 每个 TOML 表一个结构体」（`Config::ping` / `runtime` / `timing` …），
这不是为了多套一层，而是为了让**三个地方的名字完全一样**：

| 地方 | 例子 |
| --- | --- |
| 配置文件里的键 | `[runtime]` 表下的 `logs_dir` |
| 代码里读它的写法 | `config::get().runtime.logs_dir` |
| 启动校验的报错 | `配置项 runtime.logs_dir 不能为空` |

把结构体拍平成「只有具体字段的大结构体」之前，得先决定拿 TOML 的表怎么办，三条路都不划算：

* **连文件一起拍平**（`runtime_logs_dir = "logs"`）：文件与结构体仍一一对应，但
  **所有现有部署的配置都要重写**（本项目开着 `deny_unknown_fields`，旧写法会直接启动失败），
  而且 `[admins]`（QQ 号 → 名字）本来就是映射，拍不成"每项具体的配置"；
  字段还会出现重名（`kovi.debug`、`onebot.path` 与将来的同名字段），只能加前缀区分，
  文件键名与字段名从此不再同名，校验报错也没法照抄文件里的名字。
* **文件保持分表、结构体拍平**：serde 反方向做不到 —— `#[serde(flatten)]` 是"把结构体摊进文件"，
  要让 `[runtime]` 摊进一个扁平结构就只能自己写一层"按路径取值"的映射，
  代码只多不少，字段级报错也会退化成手写字符串、容易和文件漂开。
* **用宏生成"扁平视图"**：能同时保住文件和结构体，但逐字段的文档注释会变成宏参数，
  `cargo doc` 里读不到现在这种说明 —— 对这个"配置即文档"的项目不划算。

所以结论是**保持分表**：`config::get()` 之后那一层点号是可以接受的成本，
换来的正是「文件键名 = 结构体路径 = 校验报错」三者同名（第三节的表就是查这层对应关系的）。
另外「启动时读一次、之后直接取字段」这条**本来就成立**：`load()` 在最早期把配置写进 `OnceLock`，
之后所有 `config::get()` 都是内存取值、没有 IO（见第一节）。
