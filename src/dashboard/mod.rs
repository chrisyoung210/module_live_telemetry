//! Dashboard 数据服务模块
//!
//! 提供数据回传抽象（DataSink）、订阅管理与调度（DashboardService）、
//! 动态命令（DashboardCommand）以及线程化运行支持。

pub mod service;
pub mod sink;

use crate::TelemetryFrame;
use crossbeam_channel::Receiver;
use service::{DashboardCommand, DashboardService};
use std::sync::Arc;
use std::thread::JoinHandle;

/// 启动 Dashboard 服务线程
///
/// 在独立线程中运行 DashboardService，同时监听遥测帧 channel 和命令 channel。
/// - `frame_rx` 断开 → 服务退出
/// - `cmd_rx` 断开 → 服务退出
pub fn spawn_dashboard(
    mut service: DashboardService,
    frame_rx: Receiver<Arc<TelemetryFrame>>,
    cmd_rx: Receiver<DashboardCommand>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        service.run(frame_rx, cmd_rx);
    })
}
