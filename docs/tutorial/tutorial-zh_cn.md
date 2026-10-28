# 从零开始的部署教程

本教程面向首次部署本项目的用户，覆盖 Linux 环境、NapCat、配置、机器人登录、systemd 与常见问题。

## 目录

- [从零开始的部署教程](#从零开始的部署教程)
  - [目录](#目录)
  - [1. 部署前准备](#1-部署前准备)
  - [2. 安装 Linux 环境](#2-安装-linux-环境)
  - [3. 安装并登录 NapCat](#3-安装并登录-napcat)
  - [4. 放置文件与首次启动](#4-放置文件与首次启动)
  - [5. 修改配置](#5-修改配置)
  - [6. 再次启动并确认连接](#6-再次启动并确认连接)
  - [7. 设置管理员(如果已配置就跳过)](#7-设置管理员如果已配置就跳过)
  - [8. 添加并登录第一个机器人](#8-添加并登录第一个机器人)
    - [8.1 创建机器人](#81-创建机器人)
    - [8.2 登录微软账号](#82-登录微软账号)
    - [8.3 手动写入 Token（备用）](#83-手动写入-token备用)
  - [9. 确认机器人状态](#9-确认机器人状态)
  - [10. 配置 systemd](#10-配置-systemd)
  - [11. 常见问题](#11-常见问题)
  - [12. 卸载](#12-卸载)

## 1. 部署前准备

需要准备：

* 一个用于运行机器人的 QQ 账号（设置好友申请自动通过）。
* 一台 64 位 Linux 服务器。内存容量取决于同时运行的 FishingBot 实例数量（2G的机器上大概可以运行10个机器人）。
* 一个 OneBot V11 协议端，推荐 [NapCat](https://github.com/NapNeko/NapCatQQ)。
* FishingBot 的 jar，从 [FishingBot](https://github.com/MrKinau/FishingBot) 获取。
* 每个机器人对应的微软账号。

## 2. 安装 Linux 环境

连接到服务器：

```bash
ssh user@your.server.address
```

安装 `screen`：

```bash
sudo apt install screen   # Debian / Ubuntu
sudo yum install screen   # RHEL / CentOS
```

安装 Java 运行时。建议 Java 21 或 25，headless 版本即可：

```bash
sudo apt install openjdk-25-jre-headless    # Debian / Ubuntu
sudo dnf install java-25-openjdk-headless   # RHEL / CentOS
java -version
```

创建数据目录。以下示例使用 `/fishingbots`：

```bash
sudo mkdir -p /fishingbots
sudo chown "$USER" /fishingbots
cd /fishingbots
```

下载 FishingBot jar。推荐使用未来最新版（2.13.15不支持），支持 `/pause`、`/unpause` 的构建：

```bash
curl -LO https://github.com/MrKinau/FishingBot/releases/download/v2.13.15/FishingBot-2.13.15.jar
```

新配置模板中的 `[fishingbot]` 会在管理器启动时生成共享配置 `config.json`。
进服后执行的指令（例如 `/home fishing`）通过该表的 `start_text` 设置。
旧配置没有该表时仍保留手动维护游戏配置的方式。

## 3. 安装并登录 NapCat
https://napcat.napneko.icu/guide/boot/Shell
NapCat 的安装与登录方式随版本变化，请参考上方链接完成安装（建议安装cli工具），并登录 QQ 账号。
本项目不包含 NapCat，也不修改其配置。

在 NapCat 的 WebSocket 服务器配置中：

1. 启用正向 WebSocket，监听地址建议为 `127.0.0.1:3001`。
2. 如设置 access token，记录下来，稍后填入本项目配置。
3. 启用心跳上报。NapCat 的 `heartInterval` 默认值为 30000 毫秒。
4. 将运行本程序的服务器 IP 加入白名单（如启用了白名单）。

## 4. 放置文件与首次启动

将以下文件放入 `/fishingbots`（你可以更改文件夹位置）：

```text
/fishingbots/
├── mc-qq-fishbot                 # 可执行文件
├── FishingBot-具体版本.jar
└── config.json                   # 共享配置；启用 [fishingbot] 时自动生成
```

首次运行：

```bash
cd /fishingbots
./mc-qq-fishbot
```

配置文件不存在时，程序会生成 `mc-qq-fishbot.conf.toml` 并退出，退出码为 0。
终端会显示生成的配置文件路径。

## 5. 修改配置
新部署请在 `[fishingbot]` 中设置游戏服务器、自动重连与进服指令；管理器每次启动会覆盖生成 `paths.game_config`，非托管字段使用随程序发布的模板默认值。
已有部署若继续维护自己的 `config.json`，不要添加 `[fishingbot]`；迁移前请备份原文件并将需要的设置填写到该表。
本项目配置文件为 `mc-qq-fishbot.conf.toml`，字段说明见文件内注释。

提示：

* `paths.base_dir` 必须为绝对路径；`paths.jar` 与 `paths.game_config` 必须为相对路径，
  以 `base_dir` 为基准。
* `[keywords].online` 必须与游戏服务器实际进服提示一致，否则 `/status` 无法判断在线状态。
* `[startup_queue]` 默认每批 1 台、批间 20 秒，用于规避微软登录限流。完整说明见
  配置文件内注释。
* `[timing].heartbeat_timeout_ms` 必须大于 NapCat 的心跳间隔。协议端间隔为 30 秒时，
  建议设置为 90000。
* 配置仅在启动时读取一次，修改后必须重启进程。
* `/ping` 显式指定的地址必须解析为公网地址。需要探测内网服务器时，将其配置为 `[ping]` 默认目标并直接发送 `/ping`。

## 6. 再次启动并确认连接

```bash
cd /fishingbots
./mc-qq-fishbot
```

终端与本进程日志（`logs/时间戳.log`）应显示：

* `已加载配置: /fishingbots/mc-qq-fishbot.conf.toml`
* 与 OneBot 服务端的连接记录。

在 QQ 群中发送 `/help`，机器人应返回帮助菜单。如无响应，检查：

1. NapCat 是否已登录并启用正向 WebSocket；
2. `[onebot]` 的地址、端口与 access token 是否一致；
3. 本进程日志中的连接错误；
4. 机器人账号是否在群内。

## 7. 设置管理员(如果已配置就跳过)

配置文件中:
格式为“qq号" = "昵称（随意起）”

```toml
[admins]
"123456789" = "你的名字"
```

重启进程后生效。管理员可跨账号操作机器人，并使用 `/startall`、`/stopall`、`/restartall`、`/commandall`。普通用户只能操作自己 QQ 目录下的机器人。

## 8. 添加并登录第一个机器人

### 8.1 创建机器人

在 QQ 群发送：

```text
/bot add duck
```

`duck` 为示例名称，可替换。机器人名仅允许字母、数字、下划线与连字符，统一转为小写。
目录将创建在 `/fishingbots/bots/<你的 QQ 号>/duck/`。

### 8.2 登录微软账号

推荐使用 `/bot login`。先将机器人账号添加为好友，然后**私聊**发送：

```text
/bot login duck
```

程序会拉起一次 FishingBot 登录会话，随后返回微软设备码：

```text
机器人 'duck' 正在登录微软账号。
1. 打开：https://www.microsoft.com/link
2. 输入代码：XXXXXXXX
完成后请等待下一条消息（最长 10 分钟）。
```

在浏览器打开链接，登录对应微软账号并输入代码。授权成功后，程序会：

1. 保存 `refreshToken` 并将文件权限收紧为仅所有者可读写；
2. 保留当前登录会话继续运行，避免再次认证和重新排队；
3. 返回登录结果，进服状态可通过 `/status` 查看。

相关规则：

* 旧 Token 会改名为 `refreshToken.renewing`。登录失败或超时自动恢复；成功后删除备份。
* 60 秒内未获得设备码时，程序会停止登录会话并返回错误。使用 `/log duck 100` 查看登录输出，
  确认 jar 支持设备码登录，且 `[keywords].filter_regex` 未过滤相关日志。
* 登录期间，同一机器人不可执行 `/start`、`/bot rename`、`/bot del`。
* 更换 Token 前应先执行 `/stop duck`。
* 重命名、删除及手动写入 Token 前，机器人必须停止且不在启动队列中。

### 8.3 手动写入 Token（备用）

仅在无法使用设备码登录时使用：

1. 在任意机器手动运行一次 FishingBot 并完成登录；
2. 打开生成的 `refreshToken` 文件，复制全部内容；
3. 私聊机器人：

   ```text
   /token duck <token>
   ```

运行中的机器人不接受 Token 修改；修改前应执行 `/stop duck`。

## 9. 确认机器人状态

发送指令：

```text
/show status
```

返回内容包括运行中的机器人、服务器状态、暂停状态、运行时长，以及启动队列中等待或正在启动的机器人。

启动后会立即检查会话，并在 `startup_queue.verify_grace_ms` 宽限期后再次复核；会话仍存活即算拉起成功，日志只补充是否已确认进服，未匹配在线关键词不会被误判为启动失败。请确保 `[keywords].online` 与实际进服提示一致。
停止结果为「已成功停止」，因为停止流程会复核 screen 会话已消失。
`/restart` 与 `/restartall` 只重启已经运行的机器人，不会顺带启动原本停止的机器人；排队中的机器人仍应通过 `/stop` 取消。
`/pause`、`/unpause` 只有在新日志中确认对应回显后才更新状态；JAR 不支持时只提示命令已发送，不会误报成功。
`/log <名字>` 默认读取最后 100 行，也可以显式指定行数。

批量启动受启动队列节奏限制。默认每批 1 台、批间 20 秒，32 台全部启动约需 11 分钟。
等待期间可用 `/status` 查看排队明细。

常见状态问题：

* 长期显示「就绪/初始中」：`[keywords].online` 与游戏服务器实际提示不一致。
* 显示「重连/本人在线」：FishingBot 正在重连，进程仍在运行。
* 会话出现后自行退出：用 `/log` 查看机器人日志，检查 Token、jar 路径与内存。

## 10. 配置 systemd

使用 systemd 可以开机自启、统一管理进程，并在本程序退出时清理残留 JVM。

创建服务文件：

```bash
sudo nano /etc/systemd/system/mc-qq-fishbot.service
```

写入：

```ini
[Unit]
Description=mc-qq-fishbot
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/fishingbots
ExecStart=/fishingbots/mc-qq-fishbot
Restart=on-failure
RestartSec=10s

# 退出时清理整个 cgroup，避免残留 JVM 占用游戏账号。
KillMode=control-group
TimeoutStopSec=60

[Install]
WantedBy=multi-user.target
```


启用服务：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now mc-qq-fishbot
```

常用命令：

```bash
systemctl status mc-qq-fishbot
journalctl -u mc-qq-fishbot -f
sudo systemctl restart mc-qq-fishbot
sudo systemctl stop mc-qq-fishbot
```

## 11. 常见问题

| 现象                                  | 检查项                                                                                 |
| ------------------------------------- | -------------------------------------------------------------------------------------- |
| 启动后立即退出                        | `journalctl -u mc-qq-fishbot` 或终端报错；检查配置字段、类型与路径                     |
| 启动后短时间内自动停止                | NapCat 是否开启心跳；`[timing].heartbeat_timeout_ms` 是否大于心跳间隔；NapCat 连接状态 |
| 群聊指令无响应                        | QQ 号是否被风控，NapCat 连接；指令是否以 `/` 开头；机器人账号是否在群内                |
| `/bot login` 60 秒内无设备码          | `/log <名字> 100` 查看登录会话；确认 jar 支持设备码；确认 `filter_regex` 未过滤提示行  |
| 登录成功但自动启动失败                | 查看启动结果中的失败明细；检查 `java_opts`、jar 路径与内存；用 `/status` 查看会话      |
| `/start` 长时间处于排队               | `/status` 查看排队明细；按 `[startup_queue]` 的批间间隔等待                            |
| 无法停止某个机器人                    | `screen -ls` 检查会话；确认程序与 screen 会话由同一用户启动                            |
| 状态长期为「就绪/初始中」或「钓鱼中」 | `[keywords]` 与游戏服、jar 的实际输出不一致；修改后重启进程                            |
| 回复被截断                            | 调整 `[runtime].max_reply_chars`；协议端也有单条消息长度限制                           |

排查顺序：本进程日志 → 对应机器人日志 → `screen -ls` 与 `journalctl`。
机器人运行期间应使用 `/stop` 正常停止，避免直接执行 `screen -X quit`。

## 12. 卸载

停止服务后，删除部署目录即可。
程序不向部署目录之外写入文件；kovi 会在当前工作目录生成 `kovi.conf.toml` 与
`kovi.plugin.toml` 两个状态文件，可删除，本项目不读取它们。

---
