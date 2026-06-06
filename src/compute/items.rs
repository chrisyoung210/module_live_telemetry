//! 计算项 trait 定义与示例实现
//!
//! 包含：
//! - [`RealtimeComputeItem`] — 实时逐帧计算 trait
//! - [`BatchComputeItem`] — 批量整圈计算 trait
//! - [`SpeedMps`] — 静态计算项示例：速度单位转换 (km/h → m/s)
//! - [`DeltaToBestLap`] — 动态计算项示例：当前圈与参考圈的时间差

use super::{ComputeContext, ComputeError, ComputeResult};
use crate::TelemetryFrame;

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Example: Static item — SpeedMps
// ---------------------------------------------------------------------------

/// 速度单位转换：公里/小时 → 米/秒
///
/// 静态计算项，不依赖任何状态，将 `speed_kmh` 字段转换为米/秒。
pub struct SpeedMps;

impl Default for SpeedMps {
    fn default() -> Self {
        Self
    }
}

impl RealtimeComputeItem for SpeedMps {
    fn name(&self) -> &str {
        "speed_mps"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        Ok(ctx.current_frame.controls.speed_kmh as f64 / 3.6)
    }
}

// ---------------------------------------------------------------------------
// Example: Dynamic item — DeltaToBestLap
// ---------------------------------------------------------------------------

/// 当前圈与参考圈的时间差计算
///
/// 动态计算项，根据标准化赛道位置在参考圈中查找对应时间点，
/// 计算当前圈与参考圈的时间差值（毫秒）。
///
/// 使用 `last_lap_number` 和 `index` 状态变量来优化搜索效率——
/// 圈号变化时重置索引，避免每帧从头扫描。
pub struct DeltaToBestLap {
    last_lap_number: i32,
    index: usize,
}

impl Default for DeltaToBestLap {
    fn default() -> Self {
        Self {
            last_lap_number: -1,
            index: 0,
        }
    }
}

impl DeltaToBestLap {
    /// 创建新的 DeltaToBestLap 计算项
    pub fn new() -> Self {
        Self::default()
    }
}

impl RealtimeComputeItem for DeltaToBestLap {
    fn name(&self) -> &str {
        "delta_to_best_lap"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let reference = ctx.reference_lap.ok_or(ComputeError::NoValidData)?;

        if reference.is_empty() {
            return Err(ComputeError::InvalidReferenceData);
        }

        let current_lap = ctx.current_frame.session.completed_laps;
        let current_pos = ctx.current_frame.session.normalized_car_position;
        let current_time = ctx.current_frame.timing.i_current_time as f64;

        // 圈号变化时重置索引
        if current_lap != self.last_lap_number {
            self.last_lap_number = current_lap;
            self.index = 0;
        }

        // 在参考圈中查找对应位置
        // 算法：在参考圈中线性扫描，找到 normalized_car_position 落在
        // [reference[i], reference[i+1]) 内的第一个位置 i。
        // 如果同一圈内车辆倒车或位置回退，则从头重新匹配，避免沿用过高的参考索引。
        if self.index < reference.len()
            && current_pos < reference[self.index].session.normalized_car_position
        {
            self.index = 0;
        }

        for i in self.index..reference.len() {
            let ref_time = reference[i].timing.i_current_time as f64;

            if i == reference.len() - 1
                || current_pos < reference[i + 1].session.normalized_car_position
            {
                self.index = i;
                return Ok(ref_time - current_time);
            }
        }

        Err(ComputeError::ComputationFailed(
            "无法在参考圈中找到对应位置".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    };
    use std::collections::HashMap;

    fn make_frame(speed: f32, lap: i32, pos: f32, time: i32) -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: 0,
            timestamp_ns: 0,
            controls: ControlSample {
                speed_kmh: speed,
                ..ControlSample::default()
            },
            motion: MotionSample::default(),
            tyres: TyreSample::default(),
            powertrain: PowertrainSample::default(),
            session: SessionSample {
                completed_laps: lap,
                normalized_car_position: pos,
                ..SessionSample::default()
            },
            timing: TimingSample {
                i_current_time: time,
                ..TimingSample::default()
            },
            car_state: CarStateSample::default(),
            environment: EnvironmentSample::default(),
            other_cars: OtherCarsSample::default(),
        }
    }

    #[test]
    fn test_speed_mps_conversion() {
        let frame = make_frame(100.0, 1, 0.0, 0);
        let values = HashMap::new();
        let ctx = ComputeContext::new(&frame, &values);
        let mut item = SpeedMps;
        let result = item.compute(&ctx).unwrap();
        assert!((result - 27.7777).abs() < 0.01);
    }

    #[test]
    fn test_speed_mps_zero() {
        let frame = make_frame(0.0, 1, 0.0, 0);
        let values = HashMap::new();
        let ctx = ComputeContext::new(&frame, &values);
        let mut item = SpeedMps;
        let result = item.compute(&ctx).unwrap();
        assert_eq!(result, 0.0);
    }

    #[test]
    fn test_delta_to_best_lap_normal() {
        let reference = vec![
            make_frame(200.0, 1, 0.0, 0),
            make_frame(200.0, 1, 0.5, 50000),
            make_frame(200.0, 1, 1.0, 100000),
        ];
        let current = make_frame(200.0, 1, 0.5, 51000);
        let values = HashMap::new();
        let ctx = ComputeContext::with_reference(
            &current, &values, &reference,
            super::super::context::ReferenceSource {
                file_path: std::path::PathBuf::from("test.acctlm"),
                lap_number: 1,
            },
        );
        let mut item = DeltaToBestLap::new();
        let result = item.compute(&ctx).unwrap();
        assert_eq!(result, -1000.0);
    }

    #[test]
    fn test_delta_to_best_lap_reset_on_lap_change() {
        let reference = vec![
            make_frame(200.0, 1, 0.0, 0),
            make_frame(200.0, 1, 0.5, 50000),
            make_frame(200.0, 1, 1.0, 100000),
        ];
        let values = HashMap::new();

        let frame1 = make_frame(200.0, 1, 0.8, 75000);
        let ctx1 = ComputeContext::with_reference(
            &frame1, &values, &reference,
            super::super::context::ReferenceSource {
                file_path: std::path::PathBuf::from("test.acctlm"), lap_number: 1,
            },
        );
        let mut item = DeltaToBestLap::new();
        let _ = item.compute(&ctx1).unwrap();
        assert_eq!(item.index, 1);

        let frame2 = make_frame(200.0, 2, 0.1, 5000);
        let ctx2 = ComputeContext::with_reference(
            &frame2, &values, &reference,
            super::super::context::ReferenceSource {
                file_path: std::path::PathBuf::from("test.acctlm"), lap_number: 1,
            },
        );
        let result = item.compute(&ctx2).unwrap();
        assert_eq!(result, -5000.0);
        assert_eq!(item.index, 0);
    }

    #[test]
    fn test_delta_to_best_lap_resets_index_on_position_backtrack() {
        let reference = vec![
            make_frame(200.0, 1, 0.0, 0),
            make_frame(200.0, 1, 0.5, 50000),
            make_frame(200.0, 1, 1.0, 100000),
        ];
        let values = HashMap::new();
        let mut item = DeltaToBestLap::new();

        let frame1 = make_frame(200.0, 1, 0.8, 75000);
        let ctx1 = ComputeContext::with_reference(
            &frame1, &values, &reference,
            super::super::context::ReferenceSource {
                file_path: std::path::PathBuf::from("test.acctlm"), lap_number: 1,
            },
        );
        let _ = item.compute(&ctx1).unwrap();
        assert_eq!(item.index, 1);

        let frame2 = make_frame(200.0, 1, 0.4, 40000);
        let ctx2 = ComputeContext::with_reference(
            &frame2, &values, &reference,
            super::super::context::ReferenceSource {
                file_path: std::path::PathBuf::from("test.acctlm"), lap_number: 1,
            },
        );
        let result = item.compute(&ctx2).unwrap();

        assert_eq!(result, -40000.0);
        assert_eq!(item.index, 0);
    }

    #[test]
    fn test_delta_to_best_lap_no_reference() {
        let current = make_frame(200.0, 1, 0.5, 51000);
        let values = HashMap::new();
        let ctx = ComputeContext::new(&current, &values);
        let mut item = DeltaToBestLap::new();
        assert!(item.compute(&ctx).is_err());
    }

    #[test]
    fn test_delta_to_best_lap_empty_reference() {
        let current = make_frame(200.0, 1, 0.5, 51000);
        let values = HashMap::new();
        let reference: Vec<TelemetryFrame> = vec![];
        let ctx = ComputeContext::with_reference(
            &current, &values, &reference,
            super::super::context::ReferenceSource {
                file_path: std::path::PathBuf::from("test.acctlm"), lap_number: 1,
            },
        );
        let mut item = DeltaToBestLap::new();
        assert!(item.compute(&ctx).is_err());
    }
}
