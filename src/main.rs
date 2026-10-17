//! 进程入口：装配 kovi、跑起来、断开就收工退出。
//!
//! 策略是**不重连、断开即收工**：`Bot::run()` 在 OneBot 连接断开或收到退出信号时返回，
//! 本程序随即停掉全部机器人（释放游戏账号）并以非 0 退出码退出，交给 systemd 重新拉起 ——
//! 重新拉起就等于重连（不在进程内重连的原因见 [deployment.md](../docs/deployment.md)）。
//!
//! 启动顺序：
//!
//! 1. `config::load()`：读可执行文件同级目录的 `mc-qq-fishbot.conf.toml`；没有就生成模板后退出，
//!    不合法就带字段名报错退出（这一步不进 kovi，此时也还没有日志文件）；
//! 2. `init_process_log()`：终端与 `<部署目录>/logs/时间戳.log` 同时写（`src/process_log.rs`）；
//! 3. `Bot::build` + `mount_plugin_set` + `kovi::plugins!`：不用 `kovi::build_bot!` ——
//!    它内部会调 `load_local_conf()`（读写 kovi 自己的 `kovi.conf.toml`、还会改 `RUST_LOG`）
//!    并再装一次 kovi 的 logger，这两件事本项目都不需要；
//! 4. `bot.run()` 直到返回，然后停机器人、退出。

use kovi::Bot;
use kovi::ExitEvent;
use kovi::config::kovi_conf::KoviConf;
use kovi::event::id::ID;
use kovi_onebot::{Host, OneBotDriver, OneBotDriverConfig, Server};
use mc_qq_fishbot::config::{self, LoadOutcome};
use mc_qq_fishbot::{init_process_log, shutdown_all_bots, stop_all_bots_and_exit};

mod command_router;
mod heartbeat_event;

// `#[kovi::plugin]` 生成的符号以 `crate::__kovi_*` 形式被框架引用，必须重导出到 crate 根。
// 这里不使用通配符重导出：它会连带导入插件的 `main`，与二进制入口同名并触发
// `hidden_glob_reexports` 警告。
pub use command_router::{__kovi_get_plugin_info, __kovi_run_async_plugin};

/// 程序入口：配置 → 本进程日志 → 装配 kovi → 跑到断开 → 停机器人并退出。
///
/// 退出前统一停止全部机器人（占着账号在线时 FishingBot 的自动重连会与真人玩家冲突）。
/// 日志的两条线与落盘方式见 `src/process_log.rs` 与 `utils::startup`。
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 配置位置固定在可执行文件同级目录，先加载 + 校验；不通过就不进 kovi。
    let settings = match config::load()? {
        LoadOutcome::Loaded(settings) => settings,
        // 首次运行：模板已生成，提示使用者改完再启动。
        // 这是唯一还没走日志的地方 —— 配置都没加载成功，日志文件也就还没建。
        LoadOutcome::GeneratedTemplate => {
            println!(
                "未找到配置文件，已按模板生成：{}\n请按文件里的注释改成自己环境的取值（至少确认 paths.base_dir 与 paths.jar），然后重新启动。",
                config::path().display()
            );
            return Ok(());
        }
    };

    // 日志：建好日志文件并安装 logger（终端 + `<部署目录>/logs/时间戳.log`）。
    // 放在这里而不是更早：级别要看配置里的 `[kovi] debug`；放在 `Bot::build` 之前，
    // kovi 自己的日志才会走这条通道。
    init_process_log(settings.kovi.debug);

    log::info!("已加载配置: {}", config::path().display());
    log::info!("进程 PID: {}", std::process::id());

    // 刻意不做单实例保护：kovi-onebot 是 WebSocket 客户端（不占本机端口），两个实例都能连上协议端、
    // 谁都不会报错；进程也不持有权威状态（状态在 screen 会话与磁盘上）。加锁只能是"机器级"作用域，
    // 反而会挡住"换个 paths.base_dir 并行跑一份"这种正常用法（见 docs/deployment.md 第八节）。

    // 用配置里的 `[onebot]` 组装驱动；kovi 自己的配置文件交给它自己（本项目不读不写）。
    let onebot = &settings.onebot;
    let host = match onebot.host.trim().parse::<std::net::IpAddr>() {
        Ok(ip) => Host::IpAddr(ip),
        Err(_) => Host::Domain(onebot.host.trim().to_string()),
    };
    let server = Server::new(
        host,
        onebot.port,
        onebot.access_token.clone(),
        onebot.secure,
        onebot.path.clone(),
        onebot.all_in_one,
    );

    let kovi_conf = KoviConf::new(ID::new(0_i64), None, settings.kovi.debug);
    let driver = OneBotDriver::new(OneBotDriverConfig { server });

    let mut bot = Bot::build(kovi_conf, driver);
    bot.mount_plugin_set(kovi::plugins!(command_router));

    match bot.run().await {
        // 收到退出信号（systemctl stop/restart、Ctrl+C、SIGHUP）：正常收工，退出码 0，
        // systemd 不会（也不该）重启。
        ExitEvent::FromSignal => {
            log::info!("收到退出信号，正在停止全部机器人（请勿重复按 Ctrl+C）……");
            log::info!("{}", shutdown_all_bots());
            Ok(())
        }
        // 连接断开：最常见的原因是 QQ 掉线。立刻收工，并用非 0 退出码让 systemd 重新拉起；
        // 收工路径与心跳超时共用（`stop_all_bots_and_exit`）。
        ExitEvent::FromDrive => stop_all_bots_and_exit(
            "与 OneBot 服务端连接断开（通常是 QQ 掉线或 napcat 重启）：\
             正在停止全部机器人，随后退出并由 systemd 重新拉起……",
        ),
    }
}
