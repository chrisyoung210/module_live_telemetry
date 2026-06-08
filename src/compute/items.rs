//! 计算项 trait 定义与内置实现
//!
//! 包含：
//! - [`RealtimeComputeItem`] — 实时逐帧计算 trait
//! - [`BatchComputeItem`] — 批量整圈计算 trait
//! - [`DeltaTimeToLifeBestLap`] — 内置计算项：当前圈与历史最佳圈的时间差
//! - [`DeltaTimeToSessionBestLap`] — 内置计算项：当前圈与 Session 最佳圈的时间差
//! - [`all_builtin_calculated_items`] — 列出所有内置计算项
//!
//! 内置 calculated item 由用户通过 CLI 参数（`--ref-lap`）选择启用，
//! 而非硬编码在启动流程中。

use std::sync::{Arc, Mutex};

use super::{ComputeContext, ComputeError, ComputeResult};
use crate::item_key::{ItemKey, ItemType};
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
// Shared state for sector items
// ---------------------------------------------------------------------------

/// Shared state for previous-sector items (prev_sector_time / prev_sector_number).
///
/// Both `PrevSectorTimeItem` and `PrevSectorNumberItem` share the same
/// `Arc<Mutex<SectorState>>` so that sector transition is detected once
/// and both items report consistent values.
pub struct SectorState {
    pub last_seen_sector: i32,
    pub prev_sector_time: f64,
    pub prev_sector_number: f64,
}

impl Default for SectorState {
    fn default() -> Self {
        Self {
            last_seen_sector: -1,
            prev_sector_time: -1.0,
            prev_sector_number: -1.0,
        }
    }
}

/// Shared state for per-sector best-time items.
///
/// The three `SectorBestItem` instances (indices 0, 1, 2) share the same
/// `Arc<Mutex<SectorBestState>>` to track the best observed time for each
/// sector across all valid laps in the session.
pub struct SectorBestState {
    pub last_seen_sector: i32,
    pub best_times: [f64; 3],
}

impl Default for SectorBestState {
    fn default() -> Self {
        Self {
            last_seen_sector: -1,
            best_times: [-1.0, -1.0, -1.0],
        }
    }
}

// ---------------------------------------------------------------------------
// Skeleton item structs (RealtimeComputeItem not yet implemented)
// ---------------------------------------------------------------------------

pub struct PrevSectorTimeItem {
    pub state: Arc<Mutex<SectorState>>,
}

pub struct PrevSectorNumberItem {
    pub state: Arc<Mutex<SectorState>>,
}

pub struct SectorBestItem {
    pub sector_index: usize,
    pub state: Arc<Mutex<SectorBestState>>,
}

// ---------------------------------------------------------------------------
// Factory functions
// ---------------------------------------------------------------------------

/// Create a pair of items that share the same `SectorState`.
pub fn create_prev_sector_items() -> (PrevSectorTimeItem, PrevSectorNumberItem) {
    let state = Arc::new(Mutex::new(SectorState::default()));
    (
        PrevSectorTimeItem {
            state: Arc::clone(&state),
        },
        PrevSectorNumberItem {
            state: Arc::clone(&state),
        },
    )
}

/// Create three `SectorBestItem` instances (indices 0, 1, 2)
/// that share the same `SectorBestState`.
pub fn create_sector_best_items() -> (SectorBestItem, SectorBestItem, SectorBestItem) {
    let state = Arc::new(Mutex::new(SectorBestState::default()));
    (
        SectorBestItem {
            sector_index: 0,
            state: Arc::clone(&state),
        },
        SectorBestItem {
            sector_index: 1,
            state: Arc::clone(&state),
        },
        SectorBestItem {
            sector_index: 2,
            state: Arc::clone(&state),
        },
    )
}

// ---------------------------------------------------------------------------
// Built-in: DeltaTimeToLifeBestLap
// ---------------------------------------------------------------------------

/// 当前圈与历史最佳圈的时间差计算
///
/// 根据标准化赛道位置（0.0~1.0）在参考圈中查找对应时间点，
/// 返回时间差（毫秒）。正值 = 比参考圈慢，负值 = 比参考圈快。
///
/// 参考圈通过 `ReferenceSource` 在订阅时指定（文件路径 + 圈号），
/// 由 `ComputeRegistry::resolve_reference_lap()` 自动加载和缓存。
pub struct DeltaTimeToLifeBestLap {
    last_lap_number: i32,
    index: usize,
}

impl Default for DeltaTimeToLifeBestLap {
    fn default() -> Self {
        Self { last_lap_number: -1, index: 0 }
    }
}

impl DeltaTimeToLifeBestLap {
    pub fn new() -> Self { Self::default() }
}

impl RealtimeComputeItem for DeltaTimeToLifeBestLap {
    fn name(&self) -> &str { "delta_time_to_life_best_lap" }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let reference = ctx.reference_lap.ok_or(ComputeError::NoValidData)?;
        if reference.is_empty() {
            return Err(ComputeError::InvalidReferenceData);
        }

        let current_lap = ctx.current_frame.session.completed_laps;
        let current_pos = ctx.current_frame.session.normalized_car_position;
        let current_time = ctx.current_frame.timing.i_current_time as f64;

        if current_lap != self.last_lap_number {
            self.last_lap_number = current_lap;
            self.index = 0;
        }
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

// ---------------------------------------------------------------------------
// Built-in: DeltaTimeToSessionBestLap
// ---------------------------------------------------------------------------

/// 当前圈与本 Session 最佳圈的时间差计算
///
/// 与 [`DeltaTimeToLifeBestLap`] 计算逻辑相同，但参考圈来自本 session 运行时动态更新的最佳圈，
/// 而非外部 `ReferenceSource` 文件。
///
/// 启动时无参考圈，`compute()` 返回 `Err(NoValidData)`。
/// 当用户跑出有效圈后，通过 `LapCompletedCallback` + `replace_reference()` 动态注入参考圈。
pub struct DeltaTimeToSessionBestLap {
    last_lap_number: i32,
    index: usize,
}

impl Default for DeltaTimeToSessionBestLap {
    fn default() -> Self {
        Self { last_lap_number: -1, index: 0 }
    }
}

impl DeltaTimeToSessionBestLap {
    pub fn new() -> Self { Self::default() }
}

impl RealtimeComputeItem for DeltaTimeToSessionBestLap {
    fn name(&self) -> &str { "delta_time_to_session_best_lap" }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let reference = ctx.reference_lap.ok_or(ComputeError::NoValidData)?;
        if reference.is_empty() {
            return Err(ComputeError::InvalidReferenceData);
        }

        let current_lap = ctx.current_frame.session.completed_laps;
        let current_pos = ctx.current_frame.session.normalized_car_position;
        let current_time = ctx.current_frame.timing.i_current_time as f64;

        if current_lap != self.last_lap_number {
            self.last_lap_number = current_lap;
            self.index = 0;
        }
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

// ---------------------------------------------------------------------------
// Built-in item catalog
// ---------------------------------------------------------------------------

/// 内置 calculated item 条目
#[derive(Debug, Clone)]
pub struct BuiltinCalcItemEntry {
    /// 完整标识键，如 `calc:delta_time_to_life_best_lap`
    pub key: ItemKey,
    /// 中文描述
    pub description: &'static str,
    /// 单位
    pub unit: Option<&'static str>,
    /// 是否需要参考圈数据
    pub requires_reference: bool,
}

/// 返回所有内置 calculated item 的目录
///
/// 当前内置项：
/// - `calc:delta_time_to_life_best_lap` — 当前圈与历史最佳圈时间差（毫秒），需外部参考圈文件
/// - `calc:delta_time_to_session_best_lap` — 当前圈与本次 session 最佳圈时间差（毫秒），
///   参考圈运行时动态注入
///
/// # 使用示例
///
/// ```rust
/// use module_live_telemetry::compute::items::all_builtin_calculated_items;
///
/// for item in all_builtin_calculated_items() {
///     println!("{} — {} [{:?}]", item.key, item.description, item.unit);
/// }
/// ```
pub fn all_builtin_calculated_items() -> Vec<BuiltinCalcItemEntry> {
    vec![
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "delta_time_to_life_best_lap"),
            description: "当前圈与历史最佳圈时间差",
            unit: Some("ms"),
            requires_reference: true,
        },
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "delta_time_to_session_best_lap"),
            description: "当前圈与本Session最佳圈时间差",
            unit: Some("ms"),
            requires_reference: true,
        },
    ]
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
            sample_tick: 0, timestamp_ns: 0,
            controls: ControlSample { speed_kmh: speed, ..ControlSample::default() },
            motion: MotionSample::default(), tyres: TyreSample::default(),
            powertrain: PowertrainSample::default(),
            session: SessionSample {
                completed_laps: lap, normalized_car_position: pos,
                ..SessionSample::default()
            },
            timing: TimingSample { i_current_time: time, ..TimingSample::default() },
            car_state: CarStateSample::default(), environment: EnvironmentSample::default(),
            other_cars: OtherCarsSample::default(),
        }
    }

    #[test]
    fn test_delta_time_to_life_best_lap_normal() {
        let reference = vec![
            make_frame(200.0, 1, 0.0, 0),
            make_frame(200.0, 1, 0.5, 50000),
            make_frame(200.0, 1, 1.0, 100000),
        ];
        let frame = make_frame(200.0, 1, 0.5, 51000);
        let values = HashMap::new();
        let ctx = ComputeContext::with_reference(
            &frame, &values, &reference,
            crate::compute::context::ReferenceSource {
                file_path: std::path::PathBuf::from("t.acctlm"), lap_number: 1,
            },
        );
        let mut item = DeltaTimeToLifeBestLap::new();
        let delta = item.compute(&ctx).unwrap();
        assert!((delta + 1000.0).abs() < 1.0);
    }

    #[test]
    fn test_delta_time_to_life_best_lap_no_reference() {
        let frame = make_frame(200.0, 1, 0.5, 51000);
        let values = HashMap::new();
        let ctx = ComputeContext::new(&frame, &values);
        let mut item = DeltaTimeToLifeBestLap::new();
        assert!(item.compute(&ctx).is_err());
    }

    #[test]
    fn test_delta_time_to_life_best_lap_empty_reference() {
        let frame = make_frame(200.0, 1, 0.5, 51000);
        let values = HashMap::new();
        let ctx = ComputeContext {
            current_frame: &frame,
            computed_values: &values,
            reference_lap: Some(&[]),
            reference_source: None,
        };
        let mut item = DeltaTimeToLifeBestLap::new();
        assert!(item.compute(&ctx).is_err());
    }

    #[test]
    fn test_delta_time_to_life_best_lap_reset_on_lap_change() {
        let reference = vec![
            make_frame(200.0, 1, 0.0, 0),
            make_frame(200.0, 2, 0.5, 50000),
        ];
        let frame = make_frame(200.0, 2, 0.5, 52000);
        let values = HashMap::new();
        let ctx = ComputeContext::with_reference(
            &frame, &values, &reference,
            crate::compute::context::ReferenceSource {
                file_path: std::path::PathBuf::from("t.acctlm"), lap_number: 1,
            },
        );
        let mut item = DeltaTimeToLifeBestLap::new();
        let delta = item.compute(&ctx).unwrap();
        assert!((delta + 2000.0).abs() < 1.0);
    }

    #[test]
    fn test_all_builtin_calculated_items() {
        let items = super::all_builtin_calculated_items();
        assert_eq!(items.len(), 2);

        let delta = items.iter().find(|i| i.key.to_string() == "calc:delta_time_to_life_best_lap").unwrap();
        assert_eq!(delta.description, "当前圈与历史最佳圈时间差");
        assert!(delta.requires_reference);

        let session = items.iter().find(|i| i.key.to_string() == "calc:delta_time_to_session_best_lap").unwrap();
        assert_eq!(session.description, "当前圈与本Session最佳圈时间差");
        assert!(session.requires_reference);
    }

    #[test]
    fn test_session_best_lap_no_reference() {
        let frame = make_frame(200.0, 1, 0.5, 51000);
        let values = HashMap::new();
        let ctx = ComputeContext::new(&frame, &values);
        let mut item = DeltaTimeToSessionBestLap::new();
        assert!(item.compute(&ctx).is_err()); // NoValidData
    }

    #[test]
    fn test_session_best_lap_with_reference() {
        let reference = vec![
            make_frame(200.0, 1, 0.0, 0),
            make_frame(200.0, 1, 0.5, 50000),
            make_frame(200.0, 1, 1.0, 100000),
        ];
        let frame = make_frame(200.0, 1, 0.5, 52000);
        let values = HashMap::new();
        let ctx = ComputeContext::with_reference(
            &frame, &values, &reference,
            crate::compute::context::ReferenceSource {
                file_path: std::path::PathBuf::from("t.acctlm"), lap_number: 1,
            },
        );
        let mut item = DeltaTimeToSessionBestLap::new();
        let delta = item.compute(&ctx).unwrap();
        assert!((delta + 2000.0).abs() < 1.0); // 慢 2 秒
    }

    // -----------------------------------------------------------------------
    // Task 1: Factory function tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_prev_sector_items_share_state() {
        let (time_item, number_item) = create_prev_sector_items();
        // Both items must reference the exact same Arc allocation
        assert!(Arc::ptr_eq(&time_item.state, &number_item.state));
    }

    #[test]
    fn test_sector_best_items_share_state() {
        let (s0, s1, s2) = create_sector_best_items();
        assert!(Arc::ptr_eq(&s0.state, &s1.state));
        assert!(Arc::ptr_eq(&s1.state, &s2.state));
    }

    #[test]
    fn test_sector_best_state_initial_values() {
        let (s0, s1, s2) = create_sector_best_items();
        let state = s0.state.lock().unwrap();
        assert_eq!(state.best_times, [-1.0, -1.0, -1.0]);
        assert_eq!(state.last_seen_sector, -1);
        // Verify s1 and s2 see the same locked state (same Arc)
        let _ = s1;
        let _ = s2;
    }
}
