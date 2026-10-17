# 部署与运维

本文覆盖「从零把它跑起来」到「systemd 托管、故障排查」的全部内容；
各模块的实现细节见 [architecture.md](architecture.md) 的索引。

## 一、环境准备

* Linux 服务器，装有 `screen`（`apt install screen` / `yum install screen`）
  —— 每个机器人一个 screen 会话，这是进程模型的基础。
* 一个 Java 运行时：**JDK/JRE**（本项目只 `java -jar` 跑现成的 jar，用不到 javac；
  服务器上装 headless 版即可，建议 Java 25 或更新版本，例如 `apt install openjdk-25-jre-headless`）。
* 一个 OneBot V11 实现的 QQ 服务端（推荐 [NapCat](https://github.com/NapNeko/NapCatQQ)），
  已登录机器人 QQ，并配置好端口与心跳上报。
* 下载 [FishingBot](https://github.com/MrKinau/FishingBot) 的 jar，放到下面 `paths.base_dir` 里。

### 构建二进制（需要 Rust）

```bash
rustup target add x86_64-unknown-linux-musl   # 默认目标是 musl 静态链接，只需执行一次
cargo build --release
# 产物：target/x86_64-unknown-linux-musl/release/mc-qq-fishbot
```

## 二、放文件

全部放在同一个目录里（下面以 `/fishingbots` 为例，路径都能在配置里改）：

```
/fishingbots/
├── mc-qq-fishbot               # 编译产物
├── mc-qq-fishbot.conf.toml     # 首次启动自动生成
├── logs/                       # 本进程日志：每次启动一个时间戳文件（启动时自动创建）
├── bots/                       # 机器人目录，按 QQ 号分层（首次 /bot add 时自动创建）
├── FishingBot-2.13.15.jar      # FishingBot 的 jar（文件名里的版本号变了就改配置的 paths.jar）
└── config.json                 # 游戏侧配置（首次手动跑一次 jar 生成后按需修改）
```

`mc-qq-fishbot.conf.toml` 与 `logs/` 都在首次启动时自动生成；kovi 自己产生的
`kovi.conf.toml` / `kovi.plugin.toml` 也会出现在这里，**本项目不读也不写**它们。
`logs/` 需要这个目录可写：建不出日志文件时程序照常运行，只在终端留一条告警（见下文「日志在哪」）。

## 三、第一次启动

```bash
cd /fishingbots            # 二进制与配置所在的目录
./mc-qq-fishbot
```

启动顺序是固定的，**配置没准备好就不会启动任何机器人**：

1. 同级目录下没有 `mc-qq-fishbot.conf.toml`：程序按**内置模板**生成一份
   （内容与仓库里的 `mc-qq-fishbot.conf.example.toml` 一致）、打印它的路径、提示修改默认值，
   然后**退出**。按注释改好（至少 `paths.base_dir` 与 `paths.jar`）再启动一次。
2. 有配置文件：先解析 + 校验，不合法就带着字段名报错退出
   （例如 `配置项 keywords.online 不能为空`）；合法就直接进入机器人逻辑 —— **没有交互式问答**。

启动后日志里会打印 `本进程日志: <路径>`、`已加载配置: <路径>` 与 `进程 PID: <pid>`，
可以据此确认读的是哪一份配置、日志写到了哪里；这几行在终端（`journalctl`）与
`<部署目录>/logs/<时间戳>.log` 里各有一份。

## 四、权限模型

* **管理员名单**在本项目的 `[admins]` 里维护：`QQ号 = "名字"`（名字只用于 `/show admins` 展示，随便写）。
  键必须是纯数字 QQ 号，写错启动就会报错（避免权限静默失效）。
* **管理员**：可以操作任意机器人，并使用 `/startall`、`/stopall`、`/restartall`、`/commandall`。
* **普通用户**：只能操作自己 QQ 目录下的机器人。
* 名单改动后需**重启进程**生效。

## 五、部署为 systemd 服务（可选：自动重启 / 开机自启）

`/etc/systemd/system/mc-qq-fishbot.service`：

```ini
[Unit]
Description=mc-qq-fishbot (Kovi + FishingBot)
After=network.target
# 如果你的 OneBot 服务端（如 NapCat）也是 systemd 服务，可以打开下面两行、填它的服务名，
# 让协议端先起来；不需要就保持注释状态。
# Wants=napcat.service
# After=napcat.service

[Service]
Type=simple
# 工作目录的绝对路径（改成你自己的目录）：放可执行文件与 mc-qq-fishbot.conf.toml；
# kovi 的两个状态文件也会落在这里（本进程日志写在可执行文件同级的 logs/ 下，需要可写）
WorkingDirectory=/fishingbots
# 编译好的 release 二进制可执行文件绝对路径
ExecStart=/fishingbots/mc-qq-fishbot
Restart=on-failure
RestartSec=10

# 关键：机器人必须"说来停就停"，不许留占着游戏账号的孤儿 JVM。
# 这个 cgroup 里不只有本进程：screen 服务进程、它的 shell、JVM 都在里面。分两层生效：
#   1) QQ 掉线 / systemctl stop / restart：主进程先自己收（Ctrl+C → 等 JVM 走
#      Shutdown Hook 释放账号 → 兜底 screen -X quit）。正常情况到这里 cgroup 已经空了；
#   2) 主进程被 kill -9 / 崩溃 / 卡死：它已经无法收尾，只能由 systemd 收整个 cgroup ——
#      先 SIGTERM（JVM 同样会跑 Shutdown Hook 断开游戏连接），只有超过 TimeoutStopSec
#      才 SIGKILL。这正是 KillMode=process 做不到的：主进程一死就没人管 JVM 了。
# 这是 systemd 的默认值，显式写出来是因为它是"保证停机"的一部分，别被改掉。
# （也别用 mixed：那等于让 JVM 只吃 SIGKILL，停机的代价是不释放账号。）
KillMode=control-group
# 卡住的 JVM 最多让 stop/restart 多等这么久才被 SIGKILL。
# 正常收尾只要 timing.batch_stop_grace_ms + batch_stop_settle_ms（默认 3.5 秒，且与机器人数无关），
# 所以 60 秒足够；写得更大只会让卡死场景等更久。
TimeoutStopSec=60

[Install]
# 若要开机自启，需要配置此项：设置开机自启挂载的目标
WantedBy=multi-user.target
```

然后重载并启用：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now mc-qq-fishbot
```

## 六、NapCat 心跳配置（**必做**）

本项目强制要求协议端上报 `meta_event.heartbeat`：没有心跳就发现不了「连接静默死掉」
（WebSocket 半死时不会有任何断开事件），而「QQ 掉线但机器人还占着游戏账号」正是要避免的。
规则细节见 [heartbeat.md](heartbeat.md)。

* NapCat：网络配置里正向 WebSocket（`websocketServers`）的 `heartInterval`
  （单位毫秒，默认 30000，**别设成 0**）；用 WebUI 新建 WS 服务端时这一项就是「心跳周期」。
* 其它协议端：找对应的心跳间隔设置。
* 启动后一直收不到心跳会等满 `2 × 判定窗口`（默认 60 秒）后收工退出，并在日志里给出开启方式；
  修好后 systemd 会重新拉起，不需要手工干预。
* 协议端心跳间隔很大（比如 300 秒一次）时，把 `timing.heartbeat_timeout_ms` 调得比它更大
  —— 这一项同时也是「等第一个心跳」的期限基准（宽限期 = 它的 2 倍）。
* 实际生效的窗口看启动日志那行 `心跳看门狗已就绪：判定窗口 …`。

## 七、掉线时会怎样

**QQ 被踢下线 / napcat 掉线：**
napcat 会断开 WebSocket，kovi 的 `Bot::run()` 随即返回（`ExitEvent::FromDrive`），程序立刻停止
全部机器人（释放游戏账号），然后**带非 0 退出码退出**，由 systemd 的 `Restart=on-failure`
在 `RestartSec` 之后重新拉起 —— 重新拉起就等于重连。重新拉起之前 systemd 还会按
`KillMode=control-group` 把整个 cgroup 收一遍，所以哪怕这次收尾没停干净，也不会有 JVM 留在后台占着账号。

刻意**不在进程里重连**：napcat 的 WebSocket 服务端在 QQ 离线时照样接受连接，「重连成功」并不代表
QQ 在线；一旦重连上去就再也收不到掉线信号，机器人会一直占着游戏账号。重启的代价很小，状态一定干净。

停掉之后要重新钓鱼，发 `/startall` 或 `/start <名字>` 即可 —— 掉线本来就是「该停」的信号。

**QQ 掉线但 napcat 没断（静默死掉）：** 由心跳看门狗兜底，走与上一段完全相同的收工路径。
心跳里 `status.online = false`（协议端说账号不在线）的心跳**不算数**、不刷新计时：
napcat 还活着、QQ 已经掉线时，静默计时照常走满窗口，然后按「账号不在线」收工
（账号重新上线的心跳可以把它救回来）。

**systemctl stop / restart 或 Ctrl+C：** 属于正常退出，退出码 0，systemd 不会重启。

## 八、只跑一个实例

同一条命令被回复了两次 —— 几乎可以肯定是**同时跑了两个实例**：两个进程都连着同一个协议端，
各自回了一次消息。

程序**刻意不做单实例检查**（早期版本用 Linux 抽象 socket 做过一把锁，后来删掉了）：

* kovi-onebot 是 WebSocket **客户端**，不占本机端口 —— 两个实例都能正常连上协议端，谁都不会报错，
  所以"占不到端口"这条天然护栏在它身上不存在；
* 本进程不持有权威状态：机器人在不在跑看 `screen` 会话，配置、日志都在磁盘上，两个实例看到的是
  同一份事实（`/start` 也会拒绝已在运行的机器人），因此重复实例的后果就是「回复两次」，
  不会互相把状态搞乱；
* 要拦就只能拦成"机器级"（不分配置、不分部署目录），反而会挡住「换个 `paths.base_dir` 并行跑一份」
  这种正常用法。

因此这条靠人守：**一台机器、一份配置，只跑一个实例**。发现回复重了就这样查：

```bash
pgrep -af mc-qq-fishbot                    # 有几个进程、各自的启动命令是什么
ps -o pid,lstart,cmd -p $(pgrep -d, mc-qq-fishbot)   # 各自的启动时间
systemctl status mc-qq-fishbot --no-pager  # systemd 那边的情况
journalctl -u mc-qq-fishbot | grep -E '进程 PID|已加载配置'   # 启动日志（会打印 PID）
```

最常见的成因：① 手动前台跑过一次没退出，systemd 又拉了一个；② 两套部署目录各放了一份二进制、
都连同一个协议端。（正常 `systemctl restart` 不会造成这个：systemd 要把整个 cgroup 收干净
才会拉起新的。）停掉多余的那个即可。

## 九、常见问题

**配置写错了怎么排查？**
报错信息里会带字段名，例如 `配置项 keywords.online 不能为空`、
`配置项 paths.base_dir = ... 不存在或不是目录`。

**怎么加管理员？**
在 `mc-qq-fishbot.conf.toml` 的 `[admins]` 里加一行：`"QQ号" = "名字"`（名字随便写，只用于展示），
重启进程生效。管理员能跨账号操作任意机器人，并使用 `/startall`、`/stopall`、`/restartall`、`/commandall`。

**`kovi.conf.toml` / `kovi.plugin.toml` 里写的东西还有用吗？**
没有了。它们是 kovi 自己的状态文件，本项目只从 `mc-qq-fishbot.conf.toml` 读配置：管理员在 `[admins]`、
OneBot 连接在 `[onebot]`。所以在里面写什么都无效，删掉也无妨（**下次退出时还会被它重建**）。

细节（都实测过）：这两个文件是 kovi 的 `save_bot_status` 在**退出路径**上写的，不取决于有没有用
`build_bot!` —— 只要依赖 kovi，退出时就会写。`kovi.conf.toml` 里只有 kovi 自己的
`[config] main_admin/admins`（本项目不用它的管理员体系，所以是空值），`kovi.plugin.toml` 里只有各插件的
启用状态。文件名是**相对当前目录**的，所以落在部署目录（systemd 的 `WorkingDirectory` 设在那里）。
也关不掉：kovi-onebot 依赖 `kovi` 时开着默认特性，Cargo 的特性合并会把 `save_bot_status` 打开
（本仓库 `Cargo.toml` 的注释里也记着这条）。

**运行中改了配置，会不会把正在跑的机器人搞崩？**
不会。配置在启动时读进内存，之后只从内存取、不再读盘，所以在跑的进程完全不受影响 ——
改错、改坏、甚至删掉配置文件都没关系。新配置要**重启进程**才生效。
（这也是刻意不做热重载的原因：`java_opts`、`screen_prefix` 这类是「进程级」设置，
运行中改会让新老实例不一致。）

**怎么保证重启后读到的是新配置？**
重启就是重新读一次文件：唯一的配置文件 `mc-qq-fishbot.conf.toml` 从二进制同级目录读，
改完确认存盘再重启即可；启动日志会打印 `已加载配置: <路径>`，可以据此确认读的是哪一份。
万一改错了，进程会在启动阶段报错退出，而不是带着错配置跑起来 —— 所以改之前顺手备份一下最稳。

**状态一直显示「就绪/初始中」？**
说明日志关键词没匹配上。先手动跑一次游戏，观察进入服务器时打印了什么，
把 `keywords.online` 改成那句话；其他提示同理（`reconnect_prefix` / `reconnect_suffix`）。

**升级了 FishingBot 的 jar 之后要改什么？**
只改配置里的 `paths.jar`（文件名带版本号），再核对一遍 `keywords.*` 与
`runtime.java_opts`（新版可能改了参数名）。代码不用动。
另外顺手看一眼 `<机器人目录>/logs/raw/`：正常情况下里面只有一个 `log0.log` 符号链接（指向 `/dev/null`）——
如果出现了有内容的普通文件，说明新版改了日志文件名，`-logsdir` 的丢弃就失效了（会多出一份未过滤的日志；
`/log` 仍然只读我们的那份，不受影响），把新文件名补进 `GAME_LOG_FILE_NAME` 即可。

**升级程序之后启动报「配置项 xxx 未提供」？**
新增的配置项一律**必填**（只有 `timing.heartbeat_timeout_ms` 带默认值），这是刻意的：
宁可启动失败，也不带着猜出来的默认值跑。按报错里的字段名从
`mc-qq-fishbot.conf.example.toml` 抄一行过来即可（示例文件里每个字段都有注释）。

**`/startall` 和 `/restartall` 里有几个启动失败？**
批量启动的瞬时内存峰值通常高于云服务器的可用内存，被 OOM 杀掉的进程不会出现在
`screen -list` 里，因此结果里会明确写「启动指令已发送，但进程不存在（通常是内存不足）」。
调小 `runtime.java_opts` 的内存配额、或调大 `timing.batch_stagger_ms` 增加错峰时间，
或稍后再重复一次命令，JVM 使用内存会随时间优化。

**日志在哪？**

两份，都是每次启动一个新文件：

| 日志 | 位置 | 说明 |
| --- | --- | --- |
| 机器人日志 | `<机器人目录>/<runtime.logs_dir>/年-月-日-时-分-秒.log` | 只含**过滤后**的内容（`grep -v -E '<keywords.filter_regex>'` 丢掉刷屏行再 `tee` 落盘）；群里 `/log <机器人名> <行数>` 看的就是它 |
| 本进程日志 | `<部署目录>/logs/年-月-日-时-分-秒.log` | kovi、kovi-onebot 与本程序自己打印的内容；终端 / `journalctl` 里同时有一份（不自动轮转，要长期保留就自己清理） |

两份都是边跑边写（硬杀最多丢最后几 KB），所以停机流程里没有「归档」这一步；
命名与挑选口径见 [utils.md 第七节](utils.md#七日志有两条线) 与 [process-log.md](process-log.md)。

FishingBot 自己的日志**不占磁盘**：`<runtime.logs_dir>/raw/log0.log` 是一个指向 `/dev/null` 的
符号链接 —— 它建不出日志文件会直接拒绝启动，所以只能给它一个"能写、但什么都不存"的目标。

**`/log` 出来的行太少，想看被过滤掉的行？**

行数少是**预期行为**（说明过滤规则在工作）。被丢掉的行不写盘，所以只能改配置来看：
把匹配它们的那段正则从 `keywords.filter_regex` 里**删掉**（它是 `grep -v -E`：命中的行被丢弃，
用 `|` 连接多段，不能含单引号），重启进程后新会话的日志里就会有这些行。

**为什么 `/start` 只说「已发送启动命令」，`/stop` 却说「已成功停止」？**
两条指令能验证到的程度不同：拉起会话后只能确认「命令已发出、`screen` 会话已出现」，
登录、进服、开始钓鱼都要等 JVM 自己跑，所以 `/start`（与 `/restart`、`/startall`、`/restartall`）
回「已发送…命令」，要看是否真的上线用 `/status`；而停止流程是**当场复核**的
（发 Ctrl+C → 等 → 强杀 → 再 `screen -list` 复核一遍），确实确认了会话消失，
才回「已成功停止」。新增指令请沿用这条原则：能验证到哪一步，就只说到哪一步。

**JVM 会不会被连坐杀掉？**
会，而且这是刻意保证的：机器人必须「说来停就停」，不许留下还占着游戏账号的孤儿 JVM。两层收尾：

1. QQ 掉线、`systemctl stop/restart`：主进程先自己停干净（Ctrl+C → 等 JVM 走 Shutdown Hook
   释放账号 → 兜底 `screen -X quit`）；
2. 主进程被 `kill -9`、崩溃、OOM 或卡死：它已经无法收尾，由 systemd 按 `KillMode=control-group`
   收掉整个 cgroup（先 SIGTERM 让 JVM 走 Shutdown Hook，只有超过 `TimeoutStopSec` 才 SIGKILL）。

注意**不启 systemd、直接前台跑二进制时只有第一层**，主进程被 `kill -9` 就没人收尾了：
这时用 `screen -ls | grep fb_`（有残留 JVM 时 `pgrep -af "java .*-refreshToken"`）查看，
再在 QQ 里发 `/stopall` 收掉（`screen -wipe` 可清掉死会话残壳）。

**只装 JRE 能跑吗？**
能。本项目只启动现成的 jar，用不到 JDK 里的编译器；版本按 FishingBot 的要求来
（推荐 Java 25，装 headless 版 JRE 即可）。


