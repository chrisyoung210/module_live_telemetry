//! 计算项 trait 定义
//!
//! 定义实时计算项（RealtimeComputeItem）和批量计算项（BatchComputeItem）的 trait。

use super::{ComputeContext, ComputeResult};
use crate::TelemetryFrame;

/// 实时计算项 trait
///
/// 逐帧计算，可以持有内部状态（如当前圈号、遍历索引等）。
/// 每次调用 `compute` 接收当前帧和上下文，返回计算结果。
pub trait RealtimeComputeItem: Send {
    /// 计算项名称（用于注册和结果标识）
    fn name(&self) -> &str;

    /// 执行逐帧计算
    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64>;
}

/// 批量计算项 trait
///
/// 整圈批量计算，对比两圈数据的所有点位。
/// 无状态，每次调用接收完整的两圈数据。
pub trait BatchComputeItem: Send {
    /// 计算项名称
    fn name(&self) -> &str;

    /// 执行整圈批量计算
    fn compute_batch(
        &self,
        current_lap: &[TelemetryFrame],
        reference_lap: &[TelemetryFrame],
    ) -> ComputeResult<Vec<f64>>;
}
