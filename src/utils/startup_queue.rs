//! 启动队列：`/start`、`/startall`、`/restart`、`/restartall` 共用的 FIFO 队列。
//!
//! 为什么要有队列：微软登录按出口 IP 限流，多个 JVM 同时登录时后几个会失败；
//! 同时启动大量 JVM 会造成内存峰值。队列按批启动（批大小、批内间隔、批间间隔来自
//! `[startup_queue]`），每批启动后等一个复核窗口，再向后报告结果。
//!
//! * 命令侧（[`start_bot_sessions`]）：把条目登记到账本、入队、等结果，最后转成 [`Report`]。
//! * 线程侧（[`StartupQueue::run_starter`]）：等队列 → 等批间间隔 → 取一批 → 逐台启动 → 复核。
//! * `/bot login` 不走队列（见 `bot_manage::login`），它直接启动一次登录会话。
//!
//! 被 `/stopall`、进程退出清出队列的条目会以「已取消」写回账本，等待结果的指令不会永久阻塞。

use std::collections::{HashMap, VecDeque};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::config;

use super::bot_dirs::BotDirEntry;
use super::bot_log_files::read_log_lines_backwards_until;
use super::log_status::{ConnectionStatus, StatusScanner};
use super::report::Report;
use super::screen::{active_screen_sessions, is_bot_screen_running, session_name_for};
use super::shutdown::{StopTiming, stop_sessions};
use super::startup::launch_bot_session;

/// 进程内唯一的启动队列。
static STARTUP_QUEUE: OnceLock<StartupQueue> = OnceLock::new();

/// 启动线程 panic 后、重启前的等待，避免连续 panic 时重复输出。
const STARTER_RESTART_BACKOFF: Duration = Duration::from_secs(1);
const MIN_ONLINE_TIMEOUT_MS: u64 = 60_000;
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// 一次提交里每台机器人的最终结论。
#[derive(Debug, Clone, PartialEq, Eq)]
enum StartOutcomeKind {
    /// 会话存活，且本次启动日志确认已经进服。
    Succeeded,
    /// 未启动（已在运行或已在队列中等待），原因直接进明细。
    Skipped(String),
    /// 排队期间被 `/stopall` 或进程退出清理取消。
    Cancelled,
    /// 启动或复核失败，原因直接进明细。
    Failure(String),
}

/// 一台机器人的结论（带名字，便于账本收齐后直接组装报告）。
#[derive(Debug, Clone)]
struct StartOutcome {
    /// 机器人名；超时兜底的那条没有名字。
    bot_name: String,
    kind: StartOutcomeKind,
}

/// 一次启动请求的账本：提交方登记台数，队列每处理完一台写回一条结论。
#[derive(Debug)]
struct StartTicket {
    /// 本批提交的复核窗口（毫秒）；同一账本中的条目共用。
    verify_grace_ms: u64,
    state: Mutex<TicketState>,
    /// 待核对清空时唤醒等待的提交方。
    done: Condvar,
    expired: AtomicBool,
}

/// 账本内部状态。
#[derive(Debug)]
struct TicketState {
    /// 仍在等待结论的台数。
    pending_bot_count: usize,
    /// 已经写回的结论（按写回顺序）。
    outcomes: Vec<StartOutcome>,
    /// 是否已经收口（收口后不再接受结论）。
    finished: bool,
}

impl StartTicket {
    /// 建一个要收齐 `bot_count` 条结论的账本。
    fn new(bot_count: usize, verify_grace_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            verify_grace_ms,
            state: Mutex::new(TicketState {
                pending_bot_count: bot_count,
                outcomes: Vec::with_capacity(bot_count),
                finished: bot_count == 0,
            }),
            done: Condvar::new(),
            expired: AtomicBool::new(false),
        })
    }

    /// 写回一台的结论；收齐后唤醒提交方。
    fn record(&self, bot_name: &str, kind: StartOutcomeKind) {
        let mut state = self.lock_state();
        if state.finished {
            return;
        }

        state.outcomes.push(StartOutcome {
            bot_name: bot_name.to_string(),
            kind,
        });
        state.pending_bot_count = state.pending_bot_count.saturating_sub(1);
        let is_done = state.pending_bot_count == 0;
        if is_done {
            state.finished = true;
        }
        drop(state);

        if is_done {
            self.done.notify_all();
        }
    }

    /// 等本次请求收齐结论；超时则把剩下的记成一条「未确认」，避免指令线程永久阻塞。
    fn wait(&self, timeout: Duration) -> Vec<StartOutcome> {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock_state();

        while !state.finished {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (next_state, _) = self
                .done
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next_state;
        }

        if !state.finished {
            self.expired.store(true, Ordering::SeqCst);
            let pending_bot_count = state.pending_bot_count;
            state.pending_bot_count = 0;
            state.finished = true;
            state.outcomes.push(StartOutcome {
                bot_name: String::new(),
                kind: StartOutcomeKind::Failure(format!(
                    "有 {pending_bot_count} 台机器人的启动结果等待超时，请用 /status 查看状态。"
                )),
            });
        }

        std::mem::take(&mut state.outcomes)
    }

    /// 取内部状态的锁（只放数据，中毒的锁也能继续用）。
    fn lock_state(&self) -> MutexGuard<'_, TicketState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 队列里的一个待启动条目：机器人与写回结论的账本。
///
/// 条目在没写回结论的情况下离开作用域（清空队列、撤单、线程 panic 展开）时，
/// [`Drop`] 会补一条「已取消」，保证等待结果的指令一定能收到回应。
#[derive(Debug)]
struct QueuedBot {
    bot_dir_entry: BotDirEntry,
    ticket: Arc<StartTicket>,
    recorded: bool,
    cancelled: Arc<AtomicBool>,
}

impl QueuedBot {
    /// 造一个待启动条目。
    fn new(bot_dir_entry: BotDirEntry, ticket: Arc<StartTicket>) -> Self {
        Self {
            bot_dir_entry,
            ticket,
            recorded: false,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 把这一台的结论写回账本（写回后 [`Drop`] 不再补写）。
    fn record(&mut self, kind: StartOutcomeKind) {
        self.ticket.record(&self.bot_dir_entry.bot_name, kind);
        self.recorded = true;
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || self.ticket.expired.load(Ordering::SeqCst)
    }
}

impl Drop for QueuedBot {
    fn drop(&mut self) {
        if !self.recorded {
            self.ticket
                .record(&self.bot_dir_entry.bot_name, StartOutcomeKind::Cancelled);
        }
    }
}

/// 队列共享状态。
#[derive(Debug, Default)]
struct QueueState {
    /// 等待启动（先进先出）。
    waiting_bots: VecDeque<QueuedBot>,
    /// 启动线程正在处理的机器人名（正在启动或复核）。
    active_bot_names: Vec<String>,
    active_cancellations: HashMap<String, Arc<AtomicBool>>,
}

impl QueueState {
    /// 这个名字是否已经在队列里（等待中或正在启动）。
    fn contains(&self, bot_name: &str) -> bool {
        self.waiting_bots
            .iter()
            .any(|queued_bot| queued_bot.bot_dir_entry.bot_name == bot_name)
            || self.active_bot_names.iter().any(|name| name == bot_name)
    }
}

/// 启动队列：外部只与本对象交互。
pub struct StartupQueue {
    /// `[startup_queue] max_batch_size`：一批最多启动几台。
    max_batch_size: usize,
    /// `[startup_queue] bot_login_interval_ms`：批内相邻两台的间隔。
    bot_login_interval_ms: u64,
    /// `[startup_queue] batch_login_interval_ms`：批与批之间的间隔。
    batch_login_interval_ms: u64,
    /// `[startup_queue] max_pending_requests`：等待列表最多放几台。
    max_pending_requests: usize,
    /// `[startup_queue] verify_grace_ms`：等待进服日志的窗口，至少 60 秒。
    verify_grace_ms: u64,
    /// 队列状态（等待列表 + 正在处理的机器人名）。
    state: Mutex<QueueState>,
    /// 最近一次取出一批的时刻（批间间隔从它算起）。
    latest_pop_timestamp: Mutex<Instant>,
    /// 队列被写入或收到停止请求时唤醒启动线程。
    added: Condvar,
    /// 收到「停止启动」请求（`/stopall`、进程退出清理）。
    stop_requested: AtomicBool,
    /// 启动线程的启动结果：起不来时每次提交都直接失败。
    starter: OnceLock<Result<(), String>>,
}

impl StartupQueue {
    /// 取进程内唯一的启动队列：首次调用按配置创建，并启动后台线程。
    pub fn get_or_init() -> &'static Self {
        let startup_queue = STARTUP_QUEUE.get_or_init(|| {
            let settings = config::get();
            let queue_config = &settings.startup_queue;

            Self {
                max_batch_size: queue_config.max_batch_size,
                bot_login_interval_ms: queue_config.bot_login_interval_ms,
                batch_login_interval_ms: queue_config.batch_login_interval_ms,
                max_pending_requests: queue_config.max_pending_requests,
                verify_grace_ms: queue_config.verify_grace_ms.max(MIN_ONLINE_TIMEOUT_MS),
                state: Mutex::new(QueueState::default()),
                latest_pop_timestamp: Mutex::new(Instant::now()),
                added: Condvar::new(),
                stop_requested: AtomicBool::new(false),
                starter: OnceLock::new(),
            }
        });

        // 启动线程只起一次；起不来的原因留给每次提交自己报。
        let _ = startup_queue.starter.get_or_init(|| {
            thread::Builder::new()
                .name("bot-starter".to_string())
                .spawn(|| startup_queue.run_starter_forever())
                .map(|_worker| ())
                .map_err(|error| error.to_string())
        });

        startup_queue
    }

    /// 启动线程是否起来了（没起来时每次提交直接给失败报告）。
    fn starter_result(&self) -> Result<(), String> {
        self.starter.get().cloned().unwrap_or(Ok(()))
    }

    /// 把条目放进等待列表：重复的、超过容量的立即写回结论，其余排队并唤醒启动线程。
    fn add_waiting_bots(&self, queued_bots: Vec<QueuedBot>) {
        let mut state = self.lock_state();
        for mut queued_bot in queued_bots {
            let bot_name = queued_bot.bot_dir_entry.bot_name.clone();

            if state.contains(&bot_name) {
                queued_bot.record(StartOutcomeKind::Skipped(
                    "已在启动队列中，未重复提交。".to_string(),
                ));
                continue;
            }
            if state.waiting_bots.len() >= self.max_pending_requests {
                queued_bot.record(StartOutcomeKind::Failure(
                    "启动队列已满（max_pending_requests），未加入队列。".to_string(),
                ));
                continue;
            }

            state.waiting_bots.push_back(queued_bot);
        }
        drop(state);

        self.added.notify_all();
    }

    /// 启动线程主体：等队列 → 等批间间隔 → 取一批 → 启动并复核。
    fn run_starter(&self) {
        let mut is_first_batch = true;

        loop {
            self.wait_until_work();

            // 收到停止请求：取走即清除，本轮不取批。
            if self.take_stop_request() {
                is_first_batch = false;
                continue;
            }

            // 批与批之间固定等待一个间隔；进程刚启动后的第一批无需等待。
            if !is_first_batch {
                let state = self.lock_state();
                drop(
                    self.added
                        .wait_timeout_while(state, self.remaining_batch_wait(), |_| {
                            !self.has_stop_request()
                        })
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                );
            }
            is_first_batch = false;

            if self.take_stop_request() {
                continue;
            }

            let batch = self.pop_batch();
            if batch.is_empty() {
                continue;
            }

            self.start_single_batch(batch);
        }
    }

    /// 启动线程的常驻入口：`panic` 后等待并重启（release 下 `panic = abort`，不会执行此路径）。
    fn run_starter_forever(&'static self) -> ! {
        loop {
            if panic::catch_unwind(AssertUnwindSafe(|| self.run_starter())).is_err() {
                log::error!("启动队列线程异常退出，1 秒后重新启动");
                thread::sleep(STARTER_RESTART_BACKOFF);
            }
        }
    }

    /// 等队列非空或收到停止请求（两者之一成立即返回）。
    fn wait_until_work(&self) {
        let mut state = self.lock_state();
        while state.waiting_bots.is_empty() && !self.has_stop_request() {
            state = self
                .added
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// 距离下一次可以取批还要等多久。
    fn remaining_batch_wait(&self) -> Duration {
        let elapsed = self.lock_latest_pop_timestamp().elapsed();
        Duration::from_millis(self.batch_login_interval_ms).saturating_sub(elapsed)
    }

    /// 从队首取出一批，并记下取批时刻。
    fn pop_batch(&self) -> Vec<QueuedBot> {
        let mut state = self.lock_state();
        let take_count = self.max_batch_size.min(state.waiting_bots.len());
        let batch: Vec<QueuedBot> = state.waiting_bots.drain(..take_count).collect();

        if !batch.is_empty() {
            for queued_bot in &batch {
                state.active_cancellations.insert(
                    queued_bot.bot_dir_entry.bot_name.clone(),
                    Arc::clone(&queued_bot.cancelled),
                );
            }
            state.active_bot_names.extend(
                batch
                    .iter()
                    .map(|queued_bot| queued_bot.bot_dir_entry.bot_name.clone()),
            );
            *self.lock_latest_pop_timestamp() = Instant::now();
        }

        batch
    }

    /// 启动一批并复核；结束时将该批从「正在处理」中移除。
    fn start_single_batch(&self, mut batch: Vec<QueuedBot>) {
        let active_bot_names: Vec<String> = batch
            .iter()
            .map(|queued_bot| queued_bot.bot_dir_entry.bot_name.clone())
            .collect();

        let result = panic::catch_unwind(AssertUnwindSafe(|| self.run_batch(&mut batch)));
        self.finish_active(&active_bot_names);
        if let Err(payload) = result {
            panic::resume_unwind(payload);
        }
    }

    /// 逐台启动（批内错峰），最后统一复核。
    fn run_batch(&self, batch: &mut [QueuedBot]) {
        let mut launched_bots: Vec<(usize, PathBuf, Instant)> = Vec::new();

        for index in 0..batch.len() {
            // 撤单可以命中已取出的条目；检查后启动的竞态由撤单方等待本批释放来兜底。
            if batch[index].is_cancelled() {
                batch[index].record(StartOutcomeKind::Cancelled);
                self.finish_active(std::slice::from_ref(&batch[index].bot_dir_entry.bot_name));
                continue;
            }

            let bot_dir_entry = &batch[index].bot_dir_entry;
            if is_bot_screen_running(&bot_dir_entry.bot_name) {
                batch[index].record(StartOutcomeKind::Skipped(
                    "已在运行，未重复启动。".to_string(),
                ));
            } else if !bot_dir_entry.bot_token_status.is_ready() {
                batch[index].record(StartOutcomeKind::Failure(
                    "缺少 Token，未启动；请先执行 /bot login 或 /token。".to_string(),
                ));
            } else {
                match launch_bot_session(bot_dir_entry) {
                    Ok(log_path) => launched_bots.push((index, log_path, Instant::now())),
                    Err(message) => batch[index].record(StartOutcomeKind::Failure(message)),
                }
            }
            if batch[index].recorded {
                self.finish_active(std::slice::from_ref(&batch[index].bot_dir_entry.bot_name));
            }

            // 批内相邻两台也隔开：登录限流与 JVM 启动峰值都要摊平。
            let is_last_bot = index + 1 == batch.len();
            if !is_last_bot && self.bot_login_interval_ms > 0 {
                let state = self.lock_state();
                drop(
                    self.added
                        .wait_timeout_while(
                            state,
                            Duration::from_millis(self.bot_login_interval_ms),
                            |_| !batch.iter().any(QueuedBot::is_cancelled),
                        )
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                );
            }
        }

        while !launched_bots.is_empty() {
            let active_sessions = active_screen_sessions();
            launched_bots.retain(|(index, log_path, started)| {
                let queued_bot = &mut batch[*index];
                let outcome = if queued_bot.is_cancelled() {
                    Some(StartOutcomeKind::Cancelled)
                } else {
                    let mut scanner = StatusScanner::new(&config::get().keywords);
                    let log_result = read_log_lines_backwards_until(log_path, |line| {
                        scanner.feed_line(line);
                        scanner.is_decided()
                    });
                    let connection = scanner.finish().0;
                    verification_outcome(
                        session_alive(&queued_bot.bot_dir_entry.bot_name, &active_sessions),
                        connection,
                        started.elapsed()
                            >= Duration::from_millis(queued_bot.ticket.verify_grace_ms),
                        log_result.err(),
                    )
                };
                if let Some(outcome) = outcome {
                    queued_bot.record(outcome);
                    self.finish_active(std::slice::from_ref(&queued_bot.bot_dir_entry.bot_name));
                    false
                } else {
                    true
                }
            });
            if !launched_bots.is_empty() {
                let state = self.lock_state();
                drop(
                    self.added
                        .wait_timeout(state, STARTUP_POLL_INTERVAL)
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                );
            }
        }
    }

    /// 把一批名字从「正在处理」中移除。
    fn finish_active(&self, active_bot_names: &[String]) {
        let mut state = self.lock_state();
        state
            .active_bot_names
            .retain(|active_bot_name| !active_bot_names.contains(active_bot_name));
        for bot_name in active_bot_names {
            state.active_cancellations.remove(bot_name);
        }
        self.added.notify_all();
    }

    /// 这个名字是否在队列里（等待中或正在启动）。
    pub fn bot_in_queue(bot_name: &str) -> bool {
        Self::get_or_init().lock_state().contains(bot_name)
    }

    /// 等待启动的机器人名（不含正在启动的）。
    pub fn waiting_bot_names() -> Vec<String> {
        Self::get_or_init()
            .lock_state()
            .waiting_bots
            .iter()
            .map(|queued_bot| queued_bot.bot_dir_entry.bot_name.clone())
            .collect()
    }

    /// 正在启动或复核的机器人名。
    pub fn active_bot_names() -> Vec<String> {
        Self::get_or_init().lock_state().active_bot_names.clone()
    }

    /// 队列里等待 + 正在启动的总台数（`/startall` 回执用）。
    pub fn pending_bot_count() -> usize {
        let state = Self::get_or_init().lock_state();
        state.waiting_bots.len() + state.active_bot_names.len()
    }

    /// 一批最多启动几台（回执用）。
    pub fn max_batch_size() -> usize {
        Self::get_or_init().max_batch_size
    }

    /// 批内相邻两台的间隔（回执用）。
    pub fn bot_login_interval_ms() -> u64 {
        Self::get_or_init().bot_login_interval_ms
    }

    /// 批与批之间的间隔（回执用）。
    pub fn batch_login_interval_ms() -> u64 {
        Self::get_or_init().batch_login_interval_ms
    }

    /// 把一台从等待列表里撤下（`/stop` 用）；撤到了返回 `true`。
    pub fn remove_waiting_bot(bot_name: &str) -> bool {
        let mut state = Self::get_or_init().lock_state();
        let Some(position) = state
            .waiting_bots
            .iter()
            .position(|queued_bot| queued_bot.bot_dir_entry.bot_name == bot_name)
        else {
            return false;
        };

        // 移除后立即释放条目，`Drop` 会把「已取消」写回发起启动的那条指令。
        state.waiting_bots.remove(position);
        true
    }

    /// 取消等待或正在启动的条目，等启动线程不再可能创建该会话后再返回。
    pub fn cancel_bot_and_wait(bot_name: &str) -> bool {
        !Self::get_or_init()
            .cancel_and_wait(Some(bot_name))
            .is_empty()
    }

    fn cancel_and_wait(&self, bot_name: Option<&str>) -> Vec<String> {
        let mut state = self.lock_state();
        let matches = |name: &str| bot_name.is_none_or(|target| target == name);
        let mut cancelled_bot_names = Vec::new();
        state.waiting_bots.retain(|bot| {
            if matches(&bot.bot_dir_entry.bot_name) {
                cancelled_bot_names.push(bot.bot_dir_entry.bot_name.clone());
                false
            } else {
                true
            }
        });
        for (name, cancelled) in &state.active_cancellations {
            if matches(name) {
                cancelled.store(true, Ordering::SeqCst);
                cancelled_bot_names.push(name.clone());
            }
        }
        self.added.notify_all();
        while state
            .active_bot_names
            .iter()
            .any(|name| cancelled_bot_names.contains(name))
        {
            state = self
                .added
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        cancelled_bot_names
    }

    /// 清空等待列表并请求启动线程停止（用于 `/stopall` 与进程退出清理），返回被取消的机器人名。
    ///
    /// 会中断复核与错峰等待，并等待正在启动的条目释放，
    /// 调用方随后扫描 `screen` 时不会遗漏「刚启动、尚未出现在快照中」的会话。
    pub fn cancel_all_and_wait() -> Vec<String> {
        let startup_queue = Self::get_or_init();
        let cancelled_bot_names = startup_queue.cancel_and_wait(None);
        startup_queue.request_stop();
        cancelled_bot_names
    }

    /// 置位停止请求并唤醒启动线程。
    ///
    /// 置位时持有队列锁，等待中的启动线程不会遗漏该唤醒。
    fn request_stop(&self) {
        let _state = self.lock_state();
        self.stop_requested.store(true, Ordering::SeqCst);
        self.added.notify_all();
    }

    /// 取走停止请求（取到即清除，只影响本轮）。
    fn take_stop_request(&self) -> bool {
        self.stop_requested.swap(false, Ordering::SeqCst)
    }

    /// 是否有尚未取走的停止请求。
    fn has_stop_request(&self) -> bool {
        self.stop_requested.load(Ordering::SeqCst)
    }

    /// 启动结果兜底超时：按「当前队列 + 本次提交」的批数与最慢一批估算，再乘以 2 覆盖抖动。
    ///
    /// 不按 `max_pending_requests` 排满来算：否则队列空时一条 `/start` 也要等二十多分钟才
    /// 判定超时，单台故障的反馈太慢。
    fn result_timeout_for(&self, bot_count: usize) -> Duration {
        let queued_bot_count = {
            let state = self.lock_state();
            state.waiting_bots.len() + state.active_bot_names.len()
        };
        let total_bot_count = queued_bot_count.saturating_add(bot_count);
        let batch_count = total_bot_count.div_ceil(self.max_batch_size.max(1)).max(1);
        let slowest_batch_ms = self
            .batch_login_interval_ms
            .saturating_add(
                self.bot_login_interval_ms
                    .saturating_mul(self.max_batch_size.saturating_sub(1) as u64),
            )
            .saturating_add(self.verify_grace_ms);
        let slowest_start_ms = (batch_count as u64)
            .saturating_mul(slowest_batch_ms)
            .saturating_add(self.verify_grace_ms);

        Duration::from_millis(slowest_start_ms.saturating_mul(2).max(60_000))
    }

    /// 取队列状态的锁（中毒的锁也要能用：这里只放数据）。
    fn lock_state(&self) -> MutexGuard<'_, QueueState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 取「最近一次取批时刻」的锁（同上）。
    fn lock_latest_pop_timestamp(&self) -> MutexGuard<'_, Instant> {
        self.latest_pop_timestamp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 测试用构造：不读取配置、不启动线程。
    #[cfg(test)]
    fn new_for_test(
        max_batch_size: usize,
        bot_login_interval_ms: u64,
        batch_login_interval_ms: u64,
        max_pending_requests: usize,
        verify_grace_ms: u64,
    ) -> Self {
        Self {
            max_batch_size,
            bot_login_interval_ms,
            batch_login_interval_ms,
            max_pending_requests,
            verify_grace_ms,
            state: Mutex::new(QueueState::default()),
            latest_pop_timestamp: Mutex::new(Instant::now()),
            added: Condvar::new(),
            stop_requested: AtomicBool::new(false),
            starter: OnceLock::new(),
        }
    }
}

/// 提交一次启动请求：入队、等结果、转成统一报告。
///
/// 这是 `/start`、`/startall`、`/restart`、`/restartall` 唯一的启动入口；
/// 权限与 Token 前置检查由调用方完成，队列中再次校验（并发下目标可能已被其他指令启动）。
pub fn start_bot_sessions(bot_dir_entries: &[BotDirEntry]) -> Report {
    if bot_dir_entries.is_empty() {
        return Report::untitled();
    }

    let startup_queue = StartupQueue::get_or_init();
    let ticket = StartTicket::new(bot_dir_entries.len(), startup_queue.verify_grace_ms);
    // 在提交前按当前队列长度估算超时，提交后队列可能已经被取走一批。
    let result_timeout = startup_queue.result_timeout_for(bot_dir_entries.len());

    if let Err(reason) = startup_queue.starter_result() {
        for bot_dir_entry in bot_dir_entries {
            ticket.record(
                &bot_dir_entry.bot_name,
                StartOutcomeKind::Failure(format!("启动线程未能启动：{reason}")),
            );
        }
    } else {
        let queued_bots: Vec<QueuedBot> = bot_dir_entries
            .iter()
            .map(|bot_dir_entry| QueuedBot::new(bot_dir_entry.clone(), Arc::clone(&ticket)))
            .collect();
        startup_queue.add_waiting_bots(queued_bots);
    }

    build_report(ticket.wait(result_timeout))
}

/// 重启一批机器人：只停止并重新启动当前运行、且没有在启动队列中的目标。
///
/// 缺 Token 的目标**在停止之前**被排除：重启包含「先停再起」，先停止无法启动的账号会
/// 使可能在钓鱼的账号掉线。
pub fn restart_bot_sessions(bots_to_restart: &[BotDirEntry]) -> Report {
    let (running_entries, not_running_entries) =
        super::running_bots::split_bot_entries_by_running(bots_to_restart);
    let (queued_entries, running_entries): (Vec<_>, Vec<_>) = running_entries
        .into_iter()
        .partition(|entry| StartupQueue::bot_in_queue(&entry.bot_name));
    let (bot_entries_without_token, restartable_bot_entries): (Vec<BotDirEntry>, Vec<BotDirEntry>) =
        running_entries
            .into_iter()
            .partition(|bot_dir_entry| !bot_dir_entry.bot_token_status.is_ready());

    let stop_outcome = stop_sessions(
        &restartable_bot_entries,
        Vec::new(),
        StopTiming::from_config(),
    );

    // 停止后仍在运行的跳过重启，避免两个 JVM 争用同一账号。
    let bots_to_start: Vec<BotDirEntry> = restartable_bot_entries
        .iter()
        .filter(|bot_dir_entry| {
            stop_outcome
                .stopped_bot_names
                .contains(&bot_dir_entry.bot_name)
        })
        .cloned()
        .collect();

    let mut report = Report::untitled();
    report.skipped.extend(
        not_running_entries
            .iter()
            .map(|entry| format!("机器人 {}：原本未运行，无需重启。", entry.bot_name)),
    );
    report.skipped.extend(
        queued_entries
            .iter()
            .map(|entry| format!("机器人 {}：已在启动队列中，未重复重启。", entry.bot_name)),
    );
    for bot_dir_entry in &bot_entries_without_token {
        report.failed.push(format!(
            "机器人 {}：缺少 Token，重启将导致登录失败，已保持原状；请先执行 /bot login 再重启。",
            bot_dir_entry.bot_name
        ));
    }
    for bot_name in &stop_outcome.still_running_bot_names {
        report.failed.push(format!(
            "机器人 {bot_name}：停止阶段未能注销会话，已跳过重启。"
        ));
    }
    report.failed.extend(stop_outcome.failure_lines);
    report.absorb(start_bot_sessions(&bots_to_start));

    report
}

/// 把账本里的结论转成回复报告。
fn build_report(outcomes: Vec<StartOutcome>) -> Report {
    let mut report = Report::untitled();

    for outcome in outcomes {
        let prefix = if outcome.bot_name.is_empty() {
            String::new()
        } else {
            format!("机器人 {}：", outcome.bot_name)
        };

        match outcome.kind {
            StartOutcomeKind::Succeeded => report
                .succeeded
                .push(format!("{prefix}已启动，日志已确认进服。")),
            StartOutcomeKind::Skipped(reason) => report.skipped.push(format!("{prefix}{reason}")),
            StartOutcomeKind::Cancelled => report
                .skipped
                .push(format!("{prefix}已取消启动（收到停止请求）。")),
            StartOutcomeKind::Failure(reason) => report.failed.push(format!("{prefix}{reason}")),
        }
    }

    report
}

/// 会话名对应的 screen 会话是否存在。
fn session_alive(bot_name: &str, active_sessions: &HashMap<String, u32>) -> bool {
    session_name_for(bot_name)
        .is_ok_and(|target_session_name| active_sessions.contains_key(&target_session_name))
}

fn verification_outcome(
    session_alive: bool,
    connection: ConnectionStatus,
    timed_out: bool,
    log_error: Option<String>,
) -> Option<StartOutcomeKind> {
    if !session_alive {
        return Some(StartOutcomeKind::Failure(
            "启动失败：进程已退出，请检查日志。".to_string(),
        ));
    }
    if let Some(error) = log_error {
        return Some(StartOutcomeKind::Failure(format!("启动复核失败：{error}")));
    }
    match connection {
        ConnectionStatus::Stopped => Some(StartOutcomeKind::Failure(
            "启动失败：日志显示进程已结束。".to_string(),
        )),
        ConnectionStatus::Online => Some(StartOutcomeKind::Succeeded),
        _ if timed_out => Some(StartOutcomeKind::Failure(
            "启动超时：未检测到进服日志，不能确认上线；进程可能仍在运行，请检查 /status 和日志。"
                .to_string(),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::bot_dirs::BotTokenStatus;

    use super::*;

    /// 构造测试条目（无需加载配置，Token 状态直接指定）。
    fn bot(bot_name: &str) -> BotDirEntry {
        BotDirEntry {
            owner_qq: 1,
            bot_name: bot_name.to_string(),
            bot_dir_path: PathBuf::from("/nonexistent"),
            bot_token_status: BotTokenStatus::Missing,
        }
    }

    /// 账本收齐后等待方立即返回；没写回的条目在离开作用域时补「已取消」。
    #[test]
    fn a_ledger_is_done_after_every_bot_is_recorded() {
        let ticket = StartTicket::new(2, 100);
        ticket.record("duck", StartOutcomeKind::Succeeded);
        assert!(!ticket.lock_state().finished, "还差 goose");

        let queued_goose = QueuedBot::new(bot("goose"), Arc::clone(&ticket));
        drop(queued_goose);

        let outcomes = ticket.wait(Duration::from_secs(1));
        assert_eq!(outcomes.len(), 2);
        assert!(
            outcomes
                .iter()
                .any(|outcome| outcome.kind == StartOutcomeKind::Cancelled),
            "被丢弃的条目要补一条已取消"
        );
    }

    /// 等待超时后将剩余台数记为失败，调用方不会永久阻塞。
    #[test]
    fn waiting_gives_up_after_the_timeout() {
        let ticket = StartTicket::new(2, 100);
        ticket.record("duck", StartOutcomeKind::Succeeded);

        let outcomes = ticket.wait(Duration::from_millis(10));

        assert_eq!(outcomes.len(), 2);
        assert!(
            outcomes
                .iter()
                .any(|outcome| matches!(outcome.kind, StartOutcomeKind::Failure(_))),
            "没等到的那台要有一条失败说明"
        );
        assert!(ticket.lock_state().finished);
    }

    /// 报告按结论分三份：成功、跳过、失败。
    #[test]
    fn outcomes_are_grouped_into_the_report() {
        let report = build_report(vec![
            StartOutcome {
                bot_name: "duck".to_string(),
                kind: StartOutcomeKind::Succeeded,
            },
            StartOutcome {
                bot_name: "goose".to_string(),
                kind: StartOutcomeKind::Skipped("已在运行".to_string()),
            },
            StartOutcome {
                bot_name: "crane".to_string(),
                kind: StartOutcomeKind::Failure("启动失败".to_string()),
            },
        ]);

        assert_eq!(report.succeeded.len(), 1);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.failed.len(), 1);
        assert!(report.succeeded[0].contains("duck"));
        assert!(report.failed[0].contains("启动失败"));
    }

    /// 队列去重与容量：已在队列中的不重复排队，超过容量的立即写回失败。
    #[test]
    fn queue_dedupes_and_rejects_when_full() {
        let startup_queue = StartupQueue::new_for_test(1, 0, 0, 1, 0);
        let ticket = StartTicket::new(3, 0);

        startup_queue.add_waiting_bots(vec![
            QueuedBot::new(bot("duck"), Arc::clone(&ticket)),
            QueuedBot::new(bot("duck"), Arc::clone(&ticket)),
            QueuedBot::new(bot("goose"), Arc::clone(&ticket)),
        ]);

        let outcomes = ticket.wait(Duration::from_millis(10));
        assert_eq!(startup_queue.lock_state().waiting_bots.len(), 1);
        assert!(
            outcomes.iter().any(|outcome| matches!(
                &outcome.kind,
                StartOutcomeKind::Skipped(reason) if reason.contains("已在启动队列")
            )),
            "重复条目要被跳过"
        );
        assert!(
            outcomes.iter().any(|outcome| matches!(
                &outcome.kind,
                StartOutcomeKind::Failure(reason) if reason.contains("队列已满")
            )),
            "超过容量的条目应立即失败"
        );
    }

    /// 批间等待从上次取批时刻算起，过完就不再等。
    #[test]
    fn the_wait_between_batches_counts_from_the_last_pop() {
        let startup_queue = StartupQueue::new_for_test(1, 0, 20_000, 2, 0);

        *startup_queue.lock_latest_pop_timestamp() = Instant::now() - Duration::from_secs(60);
        assert_eq!(startup_queue.remaining_batch_wait(), Duration::ZERO);

        *startup_queue.lock_latest_pop_timestamp() = Instant::now() - Duration::from_secs(3);
        let remaining = startup_queue.remaining_batch_wait();
        assert!(
            remaining > Duration::from_secs(16) && remaining <= Duration::from_secs(17),
            "只等剩下的那部分: {remaining:?}"
        );
    }

    #[test]
    fn verification_needs_online_logs_and_a_live_session() {
        assert_eq!(
            verification_outcome(true, ConnectionStatus::Initializing, false, None),
            None
        );
        assert_eq!(
            verification_outcome(true, ConnectionStatus::Online, false, None),
            Some(StartOutcomeKind::Succeeded)
        );
        for (alive, connection, timeout, error) in [
            (true, ConnectionStatus::Initializing, true, None),
            (true, ConnectionStatus::Reconnecting, true, None),
            (true, ConnectionStatus::Stopped, false, None),
            (false, ConnectionStatus::Online, false, None),
            (
                true,
                ConnectionStatus::Online,
                false,
                Some("unreadable".to_string()),
            ),
        ] {
            assert!(matches!(
                verification_outcome(alive, connection, timeout, error),
                Some(StartOutcomeKind::Failure(_))
            ));
        }
    }

    #[test]
    fn cancellation_removes_only_the_requested_waiting_bot() {
        let queue = StartupQueue::new_for_test(1, 0, 0, 3, 0);
        let ticket = StartTicket::new(2, 0);
        queue.add_waiting_bots(vec![
            QueuedBot::new(bot("duck"), Arc::clone(&ticket)),
            QueuedBot::new(bot("goose"), Arc::clone(&ticket)),
        ]);
        assert_eq!(queue.cancel_and_wait(Some("duck")), vec!["duck"]);
        assert!(!queue.lock_state().contains("duck"));
        assert!(queue.lock_state().contains("goose"));
        assert_eq!(queue.cancel_and_wait(None), vec!["goose"]);
        let outcomes = ticket.wait(Duration::ZERO);
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.kind == StartOutcomeKind::Cancelled)
        );
    }

    #[test]
    fn cancelling_inflight_waits_until_the_worker_releases_the_bot() {
        let queue = Arc::new(StartupQueue::new_for_test(1, 0, 0, 1, 0));
        let ticket = StartTicket::new(1, 0);
        queue.add_waiting_bots(vec![QueuedBot::new(bot("duck"), Arc::clone(&ticket))]);
        let batch = queue.pop_batch();
        let cancelled = Arc::clone(&batch[0].cancelled);
        let worker_queue = Arc::clone(&queue);
        let worker = thread::spawn(move || worker_queue.cancel_and_wait(Some("duck")));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !cancelled.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(cancelled.load(Ordering::SeqCst));
        assert!(
            !worker.is_finished(),
            "must not race a launch still in progress"
        );
        drop(batch);
        queue.finish_active(&["duck".to_string()]);
        assert_eq!(worker.join().unwrap(), vec!["duck"]);
        assert_eq!(
            ticket.wait(Duration::ZERO)[0].kind,
            StartOutcomeKind::Cancelled
        );
    }

    #[test]
    fn expired_tickets_cannot_launch_later() {
        let ticket = StartTicket::new(1, 0);
        let queued = QueuedBot::new(bot("duck"), Arc::clone(&ticket));
        ticket.wait(Duration::ZERO);
        assert!(queued.is_cancelled());
    }

    #[test]
    fn cancelled_batch_never_launches_or_waits_for_staggering() {
        let queue = StartupQueue::new_for_test(2, 60_000, 60_000, 2, 60_000);
        let ticket = StartTicket::new(2, 60_000);
        queue.add_waiting_bots(vec![
            QueuedBot::new(bot("duck"), Arc::clone(&ticket)),
            QueuedBot::new(bot("goose"), Arc::clone(&ticket)),
        ]);
        let batch = queue.pop_batch();
        for bot in &batch {
            bot.cancelled.store(true, Ordering::SeqCst);
        }
        queue.start_single_batch(batch);
        assert!(queue.lock_state().active_bot_names.is_empty());
        assert!(
            ticket
                .wait(Duration::ZERO)
                .iter()
                .all(|outcome| { outcome.kind == StartOutcomeKind::Cancelled })
        );
    }
}
