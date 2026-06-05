//! 计算上下文
//!
//! 提供计算项执行时所需的上下文信息，包括当前帧数据、已计算值、历史参考数据等。

use crate::TelemetryFrame;
use std::collections::HashMap;
use std::path::PathBuf;

/// 参考数据来源标识
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceSource {
    /// acctlm 文件路径
    pub file_path: PathBuf,
    /// 圈号
    pub lap_number: i32,
}

/// 计算上下文
///
/// 在执行计算项时传递，包含：
/// - 当前遥测帧
/// - 已计算的其他计算项结果（按注册顺序）
/// - 历史参考圈数据
pub struct ComputeContext<'a> {
    /// 当前遥测帧
    pub current_frame: &'a TelemetryFrame,
    /// 已计算的值（按注册顺序，前面的计算项的结果）
    pub computed_values: &'a HashMap<String, f64>,
    /// 历史参考圈数据（可选）
    pub reference_lap: Option<&'a [TelemetryFrame]>,
    /// 参考数据来源（用于缓存管理和调试）
    pub reference_source: Option<ReferenceSource>,
}

impl<'a> ComputeContext<'a> {
    /// 创建新的计算上下文（无参考数据）
    pub fn new(
        current_frame: &'a TelemetryFrame,
        computed_values: &'a HashMap<String, f64>,
    ) -> Self {
        Self {
            current_frame,
            computed_values,
            reference_lap: None,
            reference_source: None,
        }
    }

    /// 创建带参考数据的计算上下文
    pub fn with_reference(
        current_frame: &'a TelemetryFrame,
        computed_values: &'a HashMap<String, f64>,
        reference_lap: &'a [TelemetryFrame],
        source: ReferenceSource,
    ) -> Self {
        Self {
            current_frame,
            computed_values,
            reference_lap: Some(reference_lap),
            reference_source: Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    };

    fn make_empty_frame() -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: 0,
            timestamp_ns: 0,
            controls: ControlSample::default(),
            motion: MotionSample::default(),
            tyres: TyreSample::default(),
            powertrain: PowertrainSample::default(),
            session: SessionSample::default(),
            timing: TimingSample::default(),
            car_state: CarStateSample::default(),
            environment: EnvironmentSample::default(),
            other_cars: OtherCarsSample::default(),
        }
    }

    #[test]
    fn test_context_new() {
        let frame = make_empty_frame();
        let values = HashMap::new();
        let ctx = ComputeContext::new(&frame, &values);
        assert_eq!(ctx.current_frame.sample_tick, 0);
        assert!(ctx.computed_values.is_empty());
        assert!(ctx.reference_lap.is_none());
    }

    #[test]
    fn test_context_with_reference() {
        let frame = make_empty_frame();
        let values = HashMap::new();
        let reference = vec![make_empty_frame()];
        let source = ReferenceSource {
            file_path: PathBuf::from("test.acctlm"),
            lap_number: 1,
        };
        let ctx = ComputeContext::with_reference(&frame, &values, &reference, source.clone());
        assert_eq!(ctx.reference_source.as_ref().unwrap().lap_number, 1);
        assert_eq!(ctx.reference_lap.unwrap().len(), 1);
    }
}
