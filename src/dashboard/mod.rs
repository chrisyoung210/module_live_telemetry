//! Dashboard 数据服务模块
//!
//! 提供数据回传抽象（DataSink）、订阅管理与调度（DashboardService）、
//! 以及线程化运行支持。

pub mod service;
pub mod sink;

use crate::TelemetryFrame;
use crossbeam_channel::Receiver;
use service::DashboardService;
use std::sync::Arc;
use std::thread::JoinHandle;

/// 启动 Dashboard 服务线程
///
/// 在独立线程中运行 DashboardService，从 `receiver` 接收遥测帧。
/// 优雅关闭：当 `receiver` 的发送端被 drop 时，线程自动退出。
pub fn spawn_dashboard(
    mut service: DashboardService,
    receiver: Receiver<Arc<TelemetryFrame>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        service.run(receiver);
    })
}
