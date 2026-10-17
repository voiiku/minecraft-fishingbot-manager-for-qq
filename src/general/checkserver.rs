//! `/checkserver`：查询钓鱼服务器自身的资源占用。

use std::time::Duration;

use sysinfo::{Disks, System};

use crate::utils::{Report, format_uptime};

/// 汇总钓鱼服务器自身的资源占用（CPU / 内存 / 磁盘 / 运行时长）。
pub async fn checkserver() -> Result<String, String> {
    let mut sys = System::new_all();

    // CPU 占用率需要两次采样求差值，中间间隔 200ms。
    sys.refresh_cpu_all();
    tokio::time::sleep(Duration::from_millis(200)).await;
    sys.refresh_cpu_all();

    let cpu_usage = sys.global_cpu_usage();

    sys.refresh_memory();
    let total_mem = sys.total_memory();
    let used_mem = sys.used_memory();
    let available_mem = sys.available_memory();

    let mem_usage_pct = if total_mem > 0 {
        (used_mem as f64 / total_mem as f64) * 100.0
    } else {
        0.0
    };
    let available_mem_mb = available_mem / (1024 * 1024);

    // 汇总所有磁盘的容量与剩余空间。
    let disks = Disks::new_with_refreshed_list();
    let total_disk: u64 = disks.iter().map(|d| d.total_space()).sum();
    let available_disk: u64 = disks.iter().map(|d| d.available_space()).sum();
    let used_disk = total_disk.saturating_sub(available_disk);

    let disk_usage_pct = if total_disk > 0 {
        (used_disk as f64 / total_disk as f64) * 100.0
    } else {
        0.0
    };

    let mut report = Report::new("钓鱼服务器状态");
    report.basics.push(format!("CPU 占用率: {cpu_usage:.1}%"));
    report
        .basics
        .push(format!("内存占用率: {mem_usage_pct:.1}%"));
    report
        .basics
        .push(format!("可用内存: {available_mem_mb} MB"));
    report
        .basics
        .push(format!("磁盘占用率: {disk_usage_pct:.1}%"));
    report.basics.push(format!(
        "服务器物理运行时间: {}",
        format_uptime(System::uptime())
    ));

    Ok(report.render())
}
