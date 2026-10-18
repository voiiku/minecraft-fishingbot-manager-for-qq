//! 心跳看门狗：根据协议端上报的 `meta_event.heartbeat` 判断对端是否在线。
//!
//! 连接被关闭（QQ 掉线时 napcat 断开 WebSocket、napcat 退出、TCP 报错）会由 kovi 报出来
//! （`main.rs` 的 `ExitEvent::FromDrive`）；WebSocket 静默失效时不会产生断开事件，因此需要心跳判定。
//! 本项目**要求协议端开启心跳上报**（NapCat 正向 WebSocket 的 `heartInterval`）：
//! 没有心跳则无法发现"连接静默退出"，所以启动后一直收不到心跳同样退出
//! （[`LostReason::NeverSeen`]）。
//!
//! 判定规则（见 [`HeartbeatWatchdog::state`]）：
//!
//! * 静默超过判定窗口 → 退出（[`LostReason::Silent`]）；
//! * 报告 `status.online = false` 的心跳不刷新计时，超窗后按账号离线处理
//!   （[`LostReason::Offline`]）；
//! * 启动后一直没收到过心跳 → 超过首次心跳宽限期 → 退出（[`LostReason::NeverSeen`]）。
//!
//! 判定窗口就是 `timing.heartbeat_timeout_ms`：它必须大于协议端的心跳间隔，
//! 否则正常心跳也会被判成掉线（NapCat 的默认间隔是 30000 毫秒）。
//!
//! 判定掉线后走与连接断开相同的退出路径（[`crate::stop_all_bots_and_exit`]）：
//! 停止全部机器人释放游戏账号 → 非 0 退出 → systemd 重新启动（即重连）。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config;

/// 首次心跳宽限期相对判定窗口的倍数：启动阶段协议端自己也可能正在启动或重连。
const FIRST_BEAT_GRACE_PER_WINDOW: u32 = 2;

/// 一次心跳里能取到的信息，由插件侧从载荷里解析（见 `command_router`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeartbeatReport {
    /// `status.online`：协议端报告机器人账号是否在线；未上报时为 `None`。
    pub online: Option<bool>,
}

impl HeartbeatReport {
    /// 这次心跳能否证明"协议端与 QQ 都正常"。
    ///
    /// 只有明确上报 `status.online = false` 才算异常（OneBot 规范里该字段表示机器人账号是否在线，
    /// NapCat 可能给出 `undefined`）；字段缺失或类型不符一律按正常处理。
    fn counts_as_alive(self) -> bool {
        self.online != Some(false)
    }
}

/// 心跳状态：仅「运行中」「等待中」「应退出」三种。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatState {
    /// 收到过有效心跳，且仍在判定窗口内。
    Alive,
    /// 尚未达到退出条件：启动后尚未收到心跳，或账号被报告不在线但还没满一个窗口。
    Waiting,
    /// 该退出了，原因见 [`LostReason`]。
    Lost(LostReason),
}

/// 判定该退出的原因，只决定提示文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LostReason {
    /// 收到过心跳，但已经静默超过判定窗口。
    Silent,
    /// 协议端一直报告账号不在线，超过判定窗口没能收到"账号在线"的心跳。
    Offline,
    /// 启动后一直没收到过心跳（协议端没上报 `meta_event.heartbeat`）。
    NeverSeen,
}

/// 心跳看门狗：只记录几个时刻，判定时由调用方传入当前时刻，便于单测。
#[derive(Debug)]
struct HeartbeatWatchdog {
    /// 创建时刻（约等于进程启动），用于计算首次心跳宽限期。
    started: Instant,
    /// 最后一次有效心跳的时刻；`None` 表示还没收到过。
    last_alive_beat: Mutex<Option<Instant>>,
    /// 本轮"报告账号不在线"的起始时刻；收到在线的报告时清除。
    offline_since: Mutex<Option<Instant>>,
}

impl HeartbeatWatchdog {
    /// 建一个初始状态的看门狗。
    fn new(started: Instant) -> Self {
        Self {
            started,
            last_alive_beat: Mutex::new(None),
            offline_since: Mutex::new(None),
        }
    }

    /// 记一次心跳。
    fn note_heartbeat(&self, report: HeartbeatReport) {
        if report.counts_as_alive() {
            *self
                .last_alive_beat
                .lock()
                .expect("心跳时刻的互斥锁不应中毒") = Some(Instant::now());
            *self.offline_since.lock().expect("离线时刻的互斥锁不应中毒") = None;
        } else {
            // 只记第一次：否则账号持续离线会不断推迟判定，始终无法达到退出条件。
            self.offline_since
                .lock()
                .expect("离线时刻的互斥锁不应中毒")
                .get_or_insert_with(Instant::now);
        }
    }

    /// 首次心跳的等待期限：窗口的 [`FIRST_BEAT_GRACE_PER_WINDOW`] 倍。
    fn first_beat_grace(&self, window: Duration) -> Duration {
        window * FIRST_BEAT_GRACE_PER_WINDOW
    }

    /// 距最后一次有效心跳过去了多久；从未收到过则为 `None`（仅用于日志）。
    fn silence(&self, now: Instant) -> Option<Duration> {
        self.last_alive_beat
            .lock()
            .expect("心跳时刻的互斥锁不应中毒")
            .map(|last_alive_beat| now.duration_since(last_alive_beat))
    }

    /// 按给定时刻与判定窗口计算状态。
    ///
    /// 收到过有效心跳就按"静默了多久"判；一次都没收到过（协议端没发心跳、或账号一直不在线）
    /// 就按"启动后等了多久"判 —— `offline_since` 用来区分这两种没收到心跳的情况。
    fn state(&self, now: Instant, window: Duration) -> HeartbeatState {
        let offline_since = *self.offline_since.lock().expect("离线时刻的互斥锁不应中毒");

        if let Some(last_alive_beat) = *self
            .last_alive_beat
            .lock()
            .expect("心跳时刻的互斥锁不应中毒")
        {
            if now.duration_since(last_alive_beat) < window {
                return HeartbeatState::Alive;
            }

            // 静默超窗：最近的心跳都在报告账号不在线时给出更准确的结论。
            return HeartbeatState::Lost(if offline_since.is_some() {
                LostReason::Offline
            } else {
                LostReason::Silent
            });
        }

        match offline_since {
            // 协议端在发心跳、但一直报告账号不在线：满足一个窗口后退出。
            Some(offline_since) if now.duration_since(offline_since) >= window => {
                HeartbeatState::Lost(LostReason::Offline)
            }
            Some(_) => HeartbeatState::Waiting,
            // 一次心跳都没收到过：超过宽限期后退出。
            None if now.duration_since(self.started) >= self.first_beat_grace(window) => {
                HeartbeatState::Lost(LostReason::NeverSeen)
            }
            None => HeartbeatState::Waiting,
        }
    }
}

/// 进程内唯一的看门狗：心跳事件与看门狗任务共用同一份状态。
fn watchdog() -> &'static HeartbeatWatchdog {
    static WATCHDOG: OnceLock<HeartbeatWatchdog> = OnceLock::new();

    WATCHDOG.get_or_init(|| HeartbeatWatchdog::new(Instant::now()))
}

/// 记一次心跳（由插件侧的心跳事件处理器调用）。
pub fn note_heartbeat(report: HeartbeatReport) {
    watchdog().note_heartbeat(report);
}

/// 当前的判定状态。
fn heartbeat_state(window: Duration) -> HeartbeatState {
    watchdog().state(Instant::now(), window)
}

/// 判定窗口：配置里的 `timing.heartbeat_timeout_ms`。
fn configured_timeout() -> Duration {
    Duration::from_millis(config::get().timing.heartbeat_timeout_ms)
}

/// 心跳看门狗任务：周期性检查，判定掉线即退出。
///
/// 由插件侧用 `kovi::spawn` 启动（长驻任务交给框架登记）；每半个判定窗口检查一次，
/// 因此最坏情况下的停机延迟是 1.5 个窗口。
///
/// 退出而非原地重连：与连接断开同一条路径，非 0 退出码交给 systemd 重新启动；
/// systemd 按 `KillMode=control-group` 回收整个 cgroup，不会留下占用游戏账号的 JVM。
pub async fn watch_heartbeat() {
    let mut previous = HeartbeatState::Waiting;

    loop {
        let window = configured_timeout();
        // 窗口极小时也应避免任务空转。
        let interval = (window / 2).max(Duration::from_millis(50));
        tokio::time::sleep(interval).await;

        let state = heartbeat_state(window);

        // 第一次收到心跳时把实际使用的参数写进日志，便于排查"为什么判/为什么没判"。
        if state == HeartbeatState::Alive && previous != HeartbeatState::Alive {
            log::info!("{}", ready_line(window));
        }
        previous = state;

        match state {
            HeartbeatState::Alive | HeartbeatState::Waiting => {}
            HeartbeatState::Lost(reason) => {
                crate::stop_all_bots_and_exit(&lost_message(reason, window))
            }
        }
    }
}

/// 判定该退出时的说明：原因不同，给出的排查方向也不同。
fn lost_message(reason: LostReason, window: Duration) -> String {
    match reason {
        LostReason::Silent => {
            let silence = watchdog()
                .silence(Instant::now())
                .unwrap_or(window)
                .as_secs_f32();
            format!(
                "已经 {silence:.0} 秒没有收到 OneBot 心跳（判定窗口 {:.0} 秒）：\
                 判定为 QQ/协议端掉线，正在停止全部机器人并退出\
                 （交由 systemd 重新拉起）……",
                window.as_secs_f32()
            )
        }
        LostReason::Offline => {
            "OneBot 心跳报告 status.online = false（协议端报告机器人账号不在线），\
             并且超过判定窗口没能收到\"账号在线\"的心跳：判定 QQ 已经掉线，\
             正在停止全部机器人并退出（交由 systemd 重新拉起）……"
                .to_string()
        }
        LostReason::NeverSeen => format!(
            "启动后 {:.0} 秒没有收到过 OneBot 心跳（meta_event.heartbeat）。\
             没有心跳就无法发现\"连接静默死掉\"（QQ 掉线时机器人会一直占着游戏账号），\
             因此这里收工（已停止全部机器人）。\n\
             请确认协议端已开启心跳上报：\n\
             \t* NapCat：网络配置里正向 WebSocket（websocketServers）的 heartInterval，\
             单位毫秒、默认 30000、不要设成 0；WebUI 新建 WS 服务端时它就是\"心跳周期\"。\n\
             \t* 其它协议端：对应的心跳间隔设置。\n\
             若协议端的心跳间隔大于判定窗口，请把 timing.heartbeat_timeout_ms\
             （当前 {:.0} 毫秒）调到比它更大；systemd 会在 RestartSec 后重新拉起，\
             协议端修好即自动恢复。",
            watchdog().first_beat_grace(window).as_secs_f32(),
            configured_timeout().as_secs_f32()
        ),
    }
}

/// 第一次收到心跳时打印的那行：看门狗实际使用的参数。
fn ready_line(window: Duration) -> String {
    format!(
        "心跳看门狗已就绪：判定窗口 {:.0} 秒（timing.heartbeat_timeout_ms = {:.0} 毫秒）。",
        window.as_secs_f32(),
        configured_timeout().as_secs_f32()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 普通心跳：协议端只上报了 `post_type` / `meta_event_type`。
    fn plain_beat() -> HeartbeatReport {
        HeartbeatReport::default()
    }

    /// 还没收到过心跳、但在宽限期内：继续等待，不据此退出。
    #[test]
    fn waiting_first_beat_keeps_waiting() {
        let started = Instant::now();
        let watchdog = HeartbeatWatchdog::new(started);
        let window = Duration::from_secs(60);

        assert_eq!(watchdog.state(started, window), HeartbeatState::Waiting);
        // 宽限期是 2 个窗口：刚过一个窗口时仍在等。
        assert_eq!(
            watchdog.state(started + window, window),
            HeartbeatState::Waiting
        );
    }

    /// 启动至今没收到过心跳、又超出宽限期：退出。
    #[test]
    fn never_beaten_beyond_grace_is_reported() {
        let started = Instant::now();
        let watchdog = HeartbeatWatchdog::new(started);
        let window = Duration::from_secs(60);

        assert_eq!(
            watchdog.state(started + window * FIRST_BEAT_GRACE_PER_WINDOW, window),
            HeartbeatState::Lost(LostReason::NeverSeen)
        );
    }

    /// 判定窗口内收到过心跳：算活着。
    #[test]
    fn recent_beat_is_alive() {
        let watchdog = HeartbeatWatchdog::new(Instant::now());
        watchdog.note_heartbeat(plain_beat());

        assert_eq!(
            watchdog.state(Instant::now(), Duration::from_secs(60)),
            HeartbeatState::Alive
        );
    }

    /// 超过判定窗口没再收到心跳：判定掉线。
    #[test]
    fn silent_after_beating_is_lost() {
        let watchdog = HeartbeatWatchdog::new(Instant::now());
        watchdog.note_heartbeat(plain_beat());

        let later = Instant::now() + Duration::from_secs(60);

        assert_eq!(
            watchdog.state(later, Duration::from_secs(60)),
            HeartbeatState::Lost(LostReason::Silent)
        );
    }

    /// 又收到心跳即恢复：判定是可逆的，不是判一次掉线就再也回不来。
    #[test]
    fn new_beat_revives_watchdog() {
        let watchdog = HeartbeatWatchdog::new(Instant::now());
        watchdog.note_heartbeat(plain_beat());

        let later = Instant::now() + Duration::from_secs(60);
        assert_eq!(
            watchdog.state(later, Duration::from_secs(60)),
            HeartbeatState::Lost(LostReason::Silent)
        );

        watchdog.note_heartbeat(plain_beat());

        assert_eq!(
            watchdog.state(Instant::now(), Duration::from_secs(60)),
            HeartbeatState::Alive
        );
    }

    /// 报告账号不在线的心跳不刷新"最后心跳时刻"：静默计时照常走满窗口
    /// （napcat 还在发心跳、QQ 已经掉线的情形）。
    #[test]
    fn offline_beat_does_not_refresh_the_timer() {
        let watchdog = HeartbeatWatchdog::new(Instant::now());
        let window = Duration::from_secs(60);
        watchdog.note_heartbeat(HeartbeatReport {
            online: Some(false),
        });

        assert_eq!(
            watchdog.state(Instant::now(), window),
            HeartbeatState::Waiting
        );
        assert_eq!(
            watchdog.state(Instant::now() + window, window),
            HeartbeatState::Lost(LostReason::Offline)
        );
    }

    /// 先在线、之后被报告离线：静默超窗后判定账号离线。
    #[test]
    fn offline_after_online_is_reported_as_offline() {
        let watchdog = HeartbeatWatchdog::new(Instant::now());
        let window = Duration::from_secs(60);
        watchdog.note_heartbeat(plain_beat());
        watchdog.note_heartbeat(HeartbeatReport {
            online: Some(false),
        });

        assert_eq!(
            watchdog.state(Instant::now() + window, window),
            HeartbeatState::Lost(LostReason::Offline)
        );
    }

    /// 账号重新上线后又算数：离线判定不是不可逆的。
    #[test]
    fn back_online_revives_watchdog() {
        let watchdog = HeartbeatWatchdog::new(Instant::now());
        let window = Duration::from_secs(60);
        watchdog.note_heartbeat(HeartbeatReport {
            online: Some(false),
        });
        assert_eq!(
            watchdog.state(Instant::now() + window, window),
            HeartbeatState::Lost(LostReason::Offline)
        );

        watchdog.note_heartbeat(HeartbeatReport { online: Some(true) });

        assert_eq!(
            watchdog.state(Instant::now(), window),
            HeartbeatState::Alive
        );
    }

    /// `status` 缺失或不是布尔（协议端差异）时不能视为"账号不在线"。
    #[test]
    fn missing_online_field_counts_as_alive() {
        let reports = [
            (HeartbeatReport::default(), true),
            (HeartbeatReport { online: Some(true) }, true),
            (
                HeartbeatReport {
                    online: Some(false),
                },
                false,
            ),
        ];

        for (report, expected) in reports {
            assert_eq!(report.counts_as_alive(), expected, "{report:?}");
        }
    }
}
