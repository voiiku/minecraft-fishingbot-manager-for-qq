# Minecraft FishingBot Manager for QQ

> **第三方项目声明**
>
> 本项目属于第三方管理工具，与 FishingBot 的原作者并无官方关联。若在使用过程中发现本项目Bug，请勿向原作者反馈，而应通过本项目的[Issues](https://github.com/voiiku/minecraft-fishingbot-manager-for-qq/issues)板块进行提交。
>
> 本项目不包含、不修改，也未分发任何 FishingBot 的源代码或二进制文件。请前往 [FishingBot 官方仓库](https://github.com/MrKinau/FishingBot) 下载官方 JAR 文件。

* **本项目不支持离线模式服务器或单人游戏。**
* **本项目仅支持运行在 64 位 Linux 系统。**
* **本项目仅运行非QQ官方机器人账号**
* `/pause` 与 `/unpause` 的逻辑已经实现，等待[FishingBot](https://github.com/MrKinau/FishingBot) 官方加入这两个命令后即可正常处理暂停状态([预计26.4](https://github.com/MrKinau/FishingBot/issues/291#issuecomment-6206998408))，其余功能不受影响。

> 共享业务逻辑参照 [Minecraft FishingBot Manager for Discord](https://github.com/voiiku/minecraft-fishingbot-manager-for-discord) 更新；本项目保留 QQ 平台的群聊、私聊与权限适配。
> 本次对齐基准：Discord `main` 的 [`bf276705`](https://github.com/voiiku/minecraft-fishingbot-manager-for-discord/commit/bf2767053b200b3714c9ab0084c510cb7b6c929d)。QQ 掉线后仍采用停止全部机器人并由 systemd 重启的策略。

## 1. 项目概要

本项目通过 QQ 群聊与私聊管理多个 Minecraft 自动钓鱼机器人。机器人本身由独立的 [FishingBot](https://github.com/MrKinau/FishingBot) 项目提供，运行 `FishingBot.jar` 需要 Java 运行时。

本项目只负责管理 FishingBot 实例，不实现钓鱼机器人的行为逻辑。

## 2. 必要说明

* 适用人群：在 Minecraft 服务器中挂机钓鱼、拥有多个游戏账号并希望集中管理的玩家。
* 不适用人群：单人游戏玩家。
* 本项目解决的问题：
* 降低分散挂机钓鱼的开销；
* 集中管理多个游戏账号；
* 提供暂停钓鱼、发送游戏内指令、查看日志、批量启动与停止等常用操作。

## 3. 开始部署

见 [docs/tutorial/tutorial-zh_cn.md](docs/tutorial/tutorial-zh_cn.md)。

## 4. 指令预览

除标注「仅私聊」的指令外，其余指令在群聊使用。私聊支持
`/help`、`/whoami`、`/bot login`、`/token`、`/log`。

所有人可用：

* `/help`：帮助菜单。文本来自配置中的 `[text].help`。
* `/whoami`：查看自己的 QQ 号，并给出可填入 `[admins]` 的一行。
* `/ping [地址] [端口]`：探测游戏服务器；自定义地址仅允许公网目标，不带参数时使用可信的 `[ping]` 配置。
* `/checkserver`：查看钓鱼服务器自身的 CPU、内存、磁盘与运行时长。
* `/status` 或 `/show status`：查看运行中的机器人及启动队列。
* `/show botconfig` 或 `/show config`：查看共享的游戏侧配置。
* `/show mybots`、`/show allbots`、`/show admins`：查看自己的机器人、全部机器人、管理员名单。
* `/bot add <机器人名>`：添加机器人并创建目录；仍需登录后才可启动。

机器人的创建者或管理员可用：

* `/bot del <机器人名>`、`/bot rename <旧名> <新名>`：删除或重命名机器人。
* `/bot login <机器人名>`：**仅私聊**；通过微软设备码登录，自动保存 Token，成功后保留当前会话继续运行。
* `/token <机器人名> <token>`：**仅私聊**；手动写入 Token，仅在无法使用设备码登录时使用。
* `/log <机器人名> [行数]`：**仅私聊**；查看最新机器人日志，默认 100 行。
* `/start`、`/stop`、`/restart`：启动、停止、重启单个机器人；重启仅针对运行中的机器人。
* `/pause`、`/unpause`：暂停钓鱼但保持在线、恢复钓鱼；只有确认 JAR 回显才更新暂停状态，不支持时只提示已发送命令。
* `/command <机器人名> <指令>`：向机器人所在游戏服务器发送指令。

仅管理员可用：

* `/startall`、`/stopall`、`/restartall`：对全部机器人批量启动、停止、重启。
* `/commandall <指令>`：向全部运行中的机器人发送同一条指令。

### 共享游戏配置

新配置模板提供 `[fishingbot]`，用于管理服务器、协议版本、自动重连、进服指令和通知设置。
启用该表时，每次启动管理器都会按模板重新生成 `paths.game_config`，不要直接修改生成文件。
旧配置未添加该表时仍使用原有游戏配置，不会覆盖文件。

## 5. 许可证

本项目原创代码采用 [MIT License](LICENSE)。

MIT 许可仅适用于本项目原创代码。第三方依赖按各自许可证分发，完整文本见
[THIRD-PARTY-NOTICES.html](THIRD-PARTY-NOTICES.html)。其中 `kovi`、`kovi-macros`、
`kovi-onebot` 使用 MPL-2.0；本项目未修改这些依赖，其源代码可从对应仓库获取。
分发二进制时请同时附带 `LICENSE` 与 `THIRD-PARTY-NOTICES.html`。

[FishingBot](https://github.com/MrKinau/FishingBot) 是使用
[GPL-3.0 许可证](https://github.com/MrKinau/FishingBot/blob/main/LICENSE) 的独立程序。
本项目只管理它，不包含它，也不与它一起分发。若将 FishingBot.jar 与本项目一起打包分发，
该分发包中的 FishingBot 部分需按 GPL-3.0 处理。
