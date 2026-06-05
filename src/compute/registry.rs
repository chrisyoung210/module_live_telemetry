//! 计算项注册中心
//!
//! 管理实时计算项和批量计算项的注册、注销和执行。
//! Implementation deferred to Task 6.

use super::items::{BatchComputeItem, RealtimeComputeItem};
use super::{ComputeContext, ComputeResult};
use crate::TelemetryFrame;
use std::collections::HashMap;

/// Placeholder for ComputeRegistry (implemented in Task 6)
#[allow(dead_code)]
pub struct ComputeRegistry {
    _placeholder: (),
}

impl ComputeRegistry {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self { _placeholder: () }
    }
}
