# heartbeat：心跳看门狗

* 代码：`src/heartbeat.rs`（判定与收工）+ `src/heartbeat_event.rs`（认 `meta_event.heartbeat` 载荷）；
  由 `command_router` 注册事件并拉起看门狗（`note_heartbeat` / `watch_heartbeat`）
* 目的：兜住**「WebSocket 静默死掉」**——对端不再发任何东西、也没有断开事件。

## 一、为什么需要它

* 连接被真正关闭（QQ 掉线时 napcat 断开 WebSocket、napcat 退出、TCP 报错）会由 kovi 报出来
  （`main.rs` 里的 `ExitEvent::FromDrive`），不需要心跳。
* 但连接半死时不会有任何事件；而 napcat 的 WebSocket 服务端在 QQ 离线时**照样接受连接**，
  所以「重连成功」并不代表 QQ 在线 —— 不在进程内重连，只能靠心跳判断对端是否还活着。

因此本项目**要求协议端开启心跳上报**：没有心跳就发现不了静默掉线，
启动后一直收不到心跳同样收工（原因码 `NeverSeen`）。

## 二、心跳从哪来

* OneBot 的 `meta_event.heartbeat`。kovi-onebot 没有导出这个事件类型，
  由 `heartbeat_event` 自己实现 `Event` 并在 `Event::de` 里认载荷，
  再调 `heartbeat::note_heartbeat(report)`（注册在 `command_router` 里）。
* 判定**不向协议端发 API 请求**（不占配额），完全是「被动接收 + 本地计时」。
* 心跳里只取 `status.online`（账号是否在线）：只有明确报 `false` 才算异常，
  字段缺失或类型不符（NapCat 可能给 `undefined`）一律当作正常心跳处理。

## 三、判定规则

| 场景 | 判定 |
| --- | --- |
| 收到有效心跳，且在窗口内 | `Alive` |
| 收到过心跳，但静默超过窗口 | 收工，原因 `Silent` |
| 协议端一直报 `status.online = false`，静默超过窗口 | 收工，原因 `Offline` |
| 启动后一直没收到过心跳，等满首次宽限期 | 收工，原因 `NeverSeen` |

* 判定窗口 = `timing.heartbeat_timeout_ms`（默认 30000，与 NapCat 默认 `heartInterval` 一致）；
  **必须大于协议端的心跳间隔**，否则正常心跳也会被判成掉线。
* 首次心跳宽限期 = 窗口 × `FIRST_BEAT_GRACE_PER_WINDOW`（2 倍）：
  启动阶段协议端自己也可能正在启动或重连，不能刚起来就判掉线。
* 「账号离线」的时刻只记**第一次**：否则一直离线会不断推迟判定、永远等不到收工。
* 判定是**可逆的**：又来有效心跳就回到 `Alive`，不是判一次就再也回不来。
* 判定时刻由调用方传入（`state(now, window)`），因此各条规则都能用单元测试覆盖。

## 四、收工路径

判定掉线后走与连接断开**完全相同**的路径（`crate::stop_all_bots_and_exit`）：

1. 停止全部机器人会话（释放游戏账号，避免 FishingBot 的自动重连和真人玩家抢号）；
2. 用非 0 退出码退出；
3. systemd 按 `Restart=on-failure` 重新拉起 —— **重新拉起就等于重连**。

收到**第一个有效心跳**时会打印一行 `心跳看门狗已就绪：判定窗口 N 秒（timing.heartbeat_timeout_ms = N 毫秒）。`
（不是启动时就打：一直收不到心跳不会出现这一行，而是走 `NeverSeen` 收工），
排查时以这一行为准（它反映的是实际生效的窗口）。

## 五、常见排查

| 现象 | 处理 |
| --- | --- |
| 进程反复重启，日志说收不到心跳 | 协议端没开心跳上报：NapCat 的 WebUI WS 服务端里把它打开（就是「心跳周期」） |
| 心跳间隔比窗口还大 | 把 `timing.heartbeat_timeout_ms` 调得比心跳间隔更大（该值同时是「等第一个心跳」的基准） |
| 从旧版本升级 | 不写 `heartbeat_timeout_ms` 也行（默认 30000）；显式写 0 仍会报错 |
