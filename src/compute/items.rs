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
// Skeleton RealtimeComputeItem impls (RED phase — stub, no real logic)
// ---------------------------------------------------------------------------

impl RealtimeComputeItem for PrevSectorTimeItem {
    fn name(&self) -> &str {
        "prev_sector_time"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let mut state = self.state.lock().unwrap();
        let current_sector_index = ctx.current_frame.session.current_sector_index;
        let is_valid_lap = ctx.current_frame.session.is_valid_lap;
        let last_sector_time = ctx.current_frame.timing.last_sector_time;

        if current_sector_index != state.last_seen_sector && state.last_seen_sector != -1 {
            state.prev_sector_time = last_sector_time as f64;
            state.prev_sector_number = state.last_seen_sector as f64;
            if is_valid_lap == 0 {
                state.prev_sector_time = -1.0;
            }
        }

        state.last_seen_sector = current_sector_index;
        Ok(state.prev_sector_time)
    }
}

impl RealtimeComputeItem for PrevSectorNumberItem {
    fn name(&self) -> &str {
        "prev_sector_number"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let mut state = self.state.lock().unwrap();
        let current_sector_index = ctx.current_frame.session.current_sector_index;
        let is_valid_lap = ctx.current_frame.session.is_valid_lap;
        let last_sector_time = ctx.current_frame.timing.last_sector_time;

        if current_sector_index != state.last_seen_sector && state.last_seen_sector != -1 {
            state.prev_sector_time = last_sector_time as f64;
            state.prev_sector_number = state.last_seen_sector as f64;
            if is_valid_lap == 0 {
                state.prev_sector_time = -1.0;
            }
        }

        state.last_seen_sector = current_sector_index;
        Ok(state.prev_sector_number)
    }
}

impl RealtimeComputeItem for SectorBestItem {
    fn name(&self) -> &str {
        match self.sector_index {
            0 => "sector_best_1",
            1 => "sector_best_2",
            2 => "sector_best_3",
            _ => unreachable!(),
        }
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let mut state = self.state.lock().unwrap();
        let current_sector = ctx.current_frame.session.current_sector_index;
        let is_valid = ctx.current_frame.session.is_valid_lap;
        let sec_time = ctx.current_frame.timing.last_sector_time;

        if current_sector != state.last_seen_sector {
            let completed = state.last_seen_sector;
            if completed >= 0 && (completed as usize) < 3 && is_valid != 0 && sec_time > 0 {
                let t = sec_time as f64;
                let idx = completed as usize;
                if state.best_times[idx] < 0.0 || t < state.best_times[idx] {
                    state.best_times[idx] = t;
                }
            }
            state.last_seen_sector = current_sector;
        }

        Ok(state.best_times[self.sector_index])
    }
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
        Self {
            last_lap_number: -1,
            index: 0,
        }
    }
}

impl DeltaTimeToLifeBestLap {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RealtimeComputeItem for DeltaTimeToLifeBestLap {
    fn name(&self) -> &str {
        "delta_time_to_life_best_lap"
    }

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
        Self {
            last_lap_number: -1,
            index: 0,
        }
    }
}

impl DeltaTimeToSessionBestLap {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RealtimeComputeItem for DeltaTimeToSessionBestLap {
    fn name(&self) -> &str {
        "delta_time_to_session_best_lap"
    }

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
                // Positive means the current lap is behind the session best.
                return Ok(current_time - ref_time);
            }
        }

        Err(ComputeError::ComputationFailed(
            "无法在参考圈中找到对应位置".into(),
        ))
    }
}

// ---------------------------------------------------------------------------

/// 当前圈相对本 Session 最佳圈的插值时间差。
///
/// 与 `DeltaTimeToSessionBestLap` 保持独立：该实现会先将参考圈整理成单调的
/// `(normalized_car_position, i_current_time)` 映射，再在相邻采样点之间线性插值。
/// 正值表示当前圈落后，负值表示当前圈领先。
pub struct DeltaTimeToSessionBestLapInterpolated {
    reference_signature: Option<(usize, usize, u64, u64)>,
    reference_points: Vec<(f64, f64)>,
}

impl Default for DeltaTimeToSessionBestLapInterpolated {
    fn default() -> Self {
        Self {
            reference_signature: None,
            reference_points: Vec::new(),
        }
    }
}

impl DeltaTimeToSessionBestLapInterpolated {
    pub fn new() -> Self {
        Self::default()
    }

    fn refresh_reference_points(&mut self, reference: &[TelemetryFrame]) {
        let signature = (
            reference.as_ptr() as usize,
            reference.len(),
            reference
                .first()
                .map(|frame| frame.sample_tick)
                .unwrap_or(0),
            reference.last().map(|frame| frame.sample_tick).unwrap_or(0),
        );
        if self.reference_signature == Some(signature) {
            return;
        }

        self.reference_signature = Some(signature);
        self.reference_points.clear();

        // If pre-lap/out-lap frames are present, prefer the segment after the
        // final start/finish wrap. Small backwards jitter is filtered below.
        let start = reference
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| {
                pair[1].session.normalized_car_position + 0.5
                    < pair[0].session.normalized_car_position
            })
            .map(|(index, _)| index + 1)
            .last()
            .unwrap_or(0);

        for frame in &reference[start..] {
            let position = frame.session.normalized_car_position as f64;
            let time_ms = frame.timing.i_current_time as f64;
            if !position.is_finite()
                || !time_ms.is_finite()
                || !(0.0..=1.0).contains(&position)
                || time_ms < 0.0
            {
                continue;
            }
            if let Some(&(last_position, last_time_ms)) = self.reference_points.last() {
                if position <= last_position || time_ms < last_time_ms {
                    continue;
                }
            }
            self.reference_points.push((position, time_ms));
        }

        if let Some(&(first_position, first_time_ms)) = self.reference_points.first() {
            if first_position > 0.0 && first_time_ms > 0.0 {
                self.reference_points.insert(0, (0.0, 0.0));
            }
        }
    }
}

impl RealtimeComputeItem for DeltaTimeToSessionBestLapInterpolated {
    fn name(&self) -> &str {
        "delta_time_to_session_best_lap_interpolated"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let reference = ctx.reference_lap.ok_or(ComputeError::NoValidData)?;
        self.refresh_reference_points(reference);
        if self.reference_points.len() < 2 {
            return Err(ComputeError::InvalidReferenceData);
        }

        let current_position = ctx.current_frame.session.normalized_car_position as f64;
        let current_time_ms = ctx.current_frame.timing.i_current_time as f64;
        if !current_position.is_finite()
            || !current_time_ms.is_finite()
            || !(0.0..=1.0).contains(&current_position)
            || current_time_ms < 0.0
        {
            return Err(ComputeError::NoValidData);
        }

        let upper = self
            .reference_points
            .partition_point(|&(position, _)| position <= current_position);
        let lower = upper.saturating_sub(1).min(self.reference_points.len() - 2);
        let (position_0, time_0) = self.reference_points[lower];
        let (position_1, time_1) = self.reference_points[lower + 1];
        let span = position_1 - position_0;
        if span <= f64::EPSILON || time_1 < time_0 {
            return Err(ComputeError::InvalidReferenceData);
        }

        let ratio = (current_position - position_0) / span;
        let reference_time_ms = time_0 + ratio * (time_1 - time_0);
        Ok(current_time_ms - reference_time_ms)
    }
}

// ---------------------------------------------------------------------------
// Car coordinates — 世界坐标提取
// ---------------------------------------------------------------------------

/// 赛车世界坐标 X（米），从 other_cars.car_coordinates[0] 提取
pub struct CarCoordX;

impl CarCoordX {
    pub fn new() -> Self {
        Self
    }
}

impl RealtimeComputeItem for CarCoordX {
    fn name(&self) -> &str {
        "car_x"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let coords = &ctx.current_frame.other_cars.car_coordinates;
        Ok(coords.first().copied().unwrap_or(0.0) as f64)
    }
}

/// 赛车世界坐标 Z（米），从 other_cars.car_coordinates[2] 提取
pub struct CarCoordZ;

impl CarCoordZ {
    pub fn new() -> Self {
        Self
    }
}

impl RealtimeComputeItem for CarCoordZ {
    fn name(&self) -> &str {
        "car_z"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let coords = &ctx.current_frame.other_cars.car_coordinates;
        Ok(coords.get(2).copied().unwrap_or(0.0) as f64)
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
/// - `calc:delta_time_to_session_best_lap_interpolated` — 当前圈与本次 session 最佳圈的
///   线性插值时间差（毫秒），参考圈运行时动态注入
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
        BuiltinCalcItemEntry {
            key: ItemKey::new(
                ItemType::Calculated,
                "delta_time_to_session_best_lap_interpolated",
            ),
            description: "当前圈与本Session最佳圈插值时间差",
            unit: Some("ms"),
            requires_reference: true,
        },
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "prev_sector_time"),
            description: "上一个Sector耗时",
            unit: Some("ms"),
            requires_reference: false,
        },
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "prev_sector_number"),
            description: "上一个Sector编号",
            unit: None,
            requires_reference: false,
        },
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "sector_best_1"),
            description: "Sector1最佳耗时",
            unit: Some("ms"),
            requires_reference: false,
        },
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "sector_best_2"),
            description: "Sector2最佳耗时",
            unit: Some("ms"),
            requires_reference: false,
        },
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "sector_best_3"),
            description: "Sector3最佳耗时",
            unit: Some("ms"),
            requires_reference: false,
        },
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "car_x"),
            description: "赛车世界坐标X",
            unit: Some("m"),
            requires_reference: false,
        },
        BuiltinCalcItemEntry {
            key: ItemKey::new(ItemType::Calculated, "car_z"),
            description: "赛车世界坐标Z",
            unit: Some("m"),
            requires_reference: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
        PowertrainSample, SessionSample, TimingSample, TyreSample,
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

    fn make_sector_frame(
        sector_index: i32,
        is_valid_lap: i32,
        last_sector_time: i32,
        completed_laps: i32,
    ) -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: 0,
            timestamp_ns: 0,
            controls: ControlSample::default(),
            motion: MotionSample::default(),
            tyres: TyreSample::default(),
            powertrain: PowertrainSample::default(),
            session: SessionSample {
                current_sector_index: sector_index,
                is_valid_lap,
                completed_laps,
                normalized_car_position: sector_index as f32 / 3.0,
                ..SessionSample::default()
            },
            timing: TimingSample {
                last_sector_time,
                ..TimingSample::default()
            },
            car_state: CarStateSample::default(),
            environment: EnvironmentSample::default(),
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
            &frame,
            &values,
            &reference,
            crate::compute::context::ReferenceSource {
                file_path: std::path::PathBuf::from("t.acctlm"),
                lap_number: 1,
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
            &frame,
            &values,
            &reference,
            crate::compute::context::ReferenceSource {
                file_path: std::path::PathBuf::from("t.acctlm"),
                lap_number: 1,
            },
        );
        let mut item = DeltaTimeToLifeBestLap::new();
        let delta = item.compute(&ctx).unwrap();
        assert!((delta + 2000.0).abs() < 1.0);
    }

    #[test]
    fn test_all_builtin_calculated_items() {
        let items = super::all_builtin_calculated_items();
        assert_eq!(items.len(), 10);

        let delta = items
            .iter()
            .find(|i| i.key.to_string() == "calc:delta_time_to_life_best_lap")
            .unwrap();
        assert_eq!(delta.description, "当前圈与历史最佳圈时间差");
        assert!(delta.requires_reference);

        let session = items
            .iter()
            .find(|i| i.key.to_string() == "calc:delta_time_to_session_best_lap")
            .unwrap();
        assert_eq!(session.description, "当前圈与本Session最佳圈时间差");
        assert!(session.requires_reference);

        let prev = items
            .iter()
            .find(|i| i.key.to_string() == "calc:prev_sector_time")
            .unwrap();
        assert_eq!(prev.unit, Some("ms"));
        assert!(!prev.requires_reference);
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
            &frame,
            &values,
            &reference,
            crate::compute::context::ReferenceSource {
                file_path: std::path::PathBuf::from("t.acctlm"),
                lap_number: 1,
            },
        );
        let mut item = DeltaTimeToSessionBestLap::new();
        let delta = item.compute(&ctx).unwrap();
        assert!((delta - 2000.0).abs() < 1.0); // 慢 2 秒
    }

    #[test]
    fn test_interpolated_session_best_lap_uses_adjacent_reference_points() {
        let reference = vec![
            make_frame(200.0, 1, 0.0, 0),
            make_frame(200.0, 1, 0.4, 40_000),
            make_frame(200.0, 1, 0.6, 62_000),
            make_frame(200.0, 1, 1.0, 100_000),
        ];
        let frame = make_frame(200.0, 2, 0.5, 52_000);
        let values = HashMap::new();
        let ctx = ComputeContext::with_reference(
            &frame,
            &values,
            &reference,
            crate::compute::context::ReferenceSource::session_best(),
        );
        let mut item = DeltaTimeToSessionBestLapInterpolated::new();

        // Reference time at 0.5 is 51,000 ms, so the current lap is 1,000 ms behind.
        assert!((item.compute(&ctx).unwrap() - 1_000.0).abs() < 0.01);
    }

    #[test]
    fn test_interpolated_session_best_lap_prefers_segment_after_wrap() {
        let reference = vec![
            make_frame(200.0, 0, 0.8, 70_000),
            make_frame(200.0, 0, 0.9, 80_000),
            make_frame(200.0, 1, 0.0, 0),
            make_frame(200.0, 1, 0.5, 50_000),
            make_frame(200.0, 1, 1.0, 100_000),
        ];
        let frame = make_frame(200.0, 2, 0.85, 86_000);
        let values = HashMap::new();
        let ctx = ComputeContext::with_reference(
            &frame,
            &values,
            &reference,
            crate::compute::context::ReferenceSource::session_best(),
        );
        let mut item = DeltaTimeToSessionBestLapInterpolated::new();

        assert!((item.compute(&ctx).unwrap() - 1_000.0).abs() < 0.01);
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

    // -----------------------------------------------------------------------
    // Task 2: PrevSectorTimeItem tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_prev_sector_time_initial_returns_minus_one() {
        // First frame: current_sector_index=0, no transition yet → -1.0
        let (mut time_item, _) = create_prev_sector_items();
        let frame = make_sector_frame(0, 1, 12345, 1);
        let values = HashMap::new();
        let ctx = ComputeContext::new(&frame, &values);
        let result = time_item.compute(&ctx).unwrap();
        assert!((result - (-1.0)).abs() < 0.01);
    }

    #[test]
    fn test_prev_sector_time_valid_lap_with_transition() {
        // Frame1: sector 0, no transition → -1.0
        // Frame2: sector 1 (transition 0→1), valid lap → last_sector_time
        let (mut time_item, _) = create_prev_sector_items();
        let values = HashMap::new();

        let f1 = make_sector_frame(0, 1, 10000, 1);
        let ctx1 = ComputeContext::new(&f1, &values);
        let r1 = time_item.compute(&ctx1).unwrap();
        assert!((r1 - (-1.0)).abs() < 0.01);

        let f2 = make_sector_frame(1, 1, 30456, 1);
        let ctx2 = ComputeContext::new(&f2, &values);
        let r2 = time_item.compute(&ctx2).unwrap();
        assert!((r2 - 30456.0).abs() < 0.01);
    }

    #[test]
    fn test_prev_sector_time_invalid_lap_returns_minus_one() {
        let (mut time_item, _) = create_prev_sector_items();
        let values = HashMap::new();

        let f1 = make_sector_frame(0, 0, 10000, 1);
        let ctx1 = ComputeContext::new(&f1, &values);
        time_item.compute(&ctx1).unwrap(); // establish baseline

        let f2 = make_sector_frame(1, 0, 30456, 1); // is_valid_lap=0
        let ctx2 = ComputeContext::new(&f2, &values);
        let r2 = time_item.compute(&ctx2).unwrap();
        assert!((r2 - (-1.0)).abs() < 0.01); // invalid lap → -1.0
    }

    #[test]
    fn test_prev_sector_time_multiple_transitions() {
        let (mut time_item, _) = create_prev_sector_items();
        let values = HashMap::new();

        // 0→1
        let f1 = make_sector_frame(0, 1, 0, 1);
        time_item
            .compute(&ComputeContext::new(&f1, &values))
            .unwrap();
        let f2 = make_sector_frame(1, 1, 25000, 1);
        let r2 = time_item
            .compute(&ComputeContext::new(&f2, &values))
            .unwrap();
        assert!((r2 - 25000.0).abs() < 0.01);

        // 1→2
        let f3 = make_sector_frame(2, 1, 30000, 1);
        let r3 = time_item
            .compute(&ComputeContext::new(&f3, &values))
            .unwrap();
        assert!((r3 - 30000.0).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Task 2: PrevSectorNumberItem tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_prev_sector_number_initial_returns_minus_one() {
        let (_, mut num_item) = create_prev_sector_items();
        let frame = make_sector_frame(0, 1, 0, 1);
        let values = HashMap::new();
        let ctx = ComputeContext::new(&frame, &values);
        let result = num_item.compute(&ctx).unwrap();
        assert!((result - (-1.0)).abs() < 0.01);
    }

    #[test]
    fn test_prev_sector_number_after_transition() {
        let (mut time_item, mut num_item) = create_prev_sector_items();
        let values = HashMap::new();

        let f1 = make_sector_frame(0, 1, 0, 1);
        time_item
            .compute(&ComputeContext::new(&f1, &values))
            .unwrap();
        num_item
            .compute(&ComputeContext::new(&f1, &values))
            .unwrap();

        // Transition 0→1: previous sector was 0
        let f2 = make_sector_frame(1, 1, 25000, 1);
        time_item
            .compute(&ComputeContext::new(&f2, &values))
            .unwrap();
        let num_r2 = num_item
            .compute(&ComputeContext::new(&f2, &values))
            .unwrap();
        assert!((num_r2 - 0.0).abs() < 0.01); // sector 0 just completed
    }

    #[test]
    fn test_prev_sector_number_sync_with_time() {
        let (mut time_item, mut num_item) = create_prev_sector_items();
        let values = HashMap::new();

        // Transition 0→1
        let f1 = make_sector_frame(0, 1, 0, 1);
        time_item
            .compute(&ComputeContext::new(&f1, &values))
            .unwrap();
        num_item
            .compute(&ComputeContext::new(&f1, &values))
            .unwrap();

        let f2 = make_sector_frame(1, 1, 30456, 1);
        let time_r = time_item
            .compute(&ComputeContext::new(&f2, &values))
            .unwrap();
        let num_r = num_item
            .compute(&ComputeContext::new(&f2, &values))
            .unwrap();
        assert!((time_r - 30456.0).abs() < 0.01);
        assert!((num_r - 0.0).abs() < 0.01);

        // Transition 1→2
        let f3 = make_sector_frame(2, 1, 18000, 1);
        let time_r3 = time_item
            .compute(&ComputeContext::new(&f3, &values))
            .unwrap();
        let num_r3 = num_item
            .compute(&ComputeContext::new(&f3, &values))
            .unwrap();
        assert!((time_r3 - 18000.0).abs() < 0.01);
        assert!((num_r3 - 1.0).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Task 2: SectorBestItem tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_sector_best_initial_all_minus_one() {
        let (s0, s1, s2) = create_sector_best_items();
        let values = HashMap::new();
        let frame = make_sector_frame(0, 1, 0, 1);
        let ctx = ComputeContext::new(&frame, &values);

        let mut s0 = s0;
        let mut s1 = s1;
        let mut s2 = s2;
        assert!((s0.compute(&ctx).unwrap() - (-1.0)).abs() < 0.01);
        assert!((s1.compute(&ctx).unwrap() - (-1.0)).abs() < 0.01);
        assert!((s2.compute(&ctx).unwrap() - (-1.0)).abs() < 0.01);
    }

    #[test]
    fn test_sector_best_updates_on_valid_transition() {
        let (mut s0, mut s1, _s2) = create_sector_best_items();
        let values = HashMap::new();

        // Frame 1: sector 0, no transition
        let f1 = make_sector_frame(0, 1, 30000, 1);
        let ctx1 = ComputeContext::new(&f1, &values);
        s0.compute(&ctx1).unwrap();
        s1.compute(&ctx1).unwrap();

        // Frame 2: sector 1 (transition 0→1), valid lap
        let f2 = make_sector_frame(1, 1, 25000, 1);
        let ctx2 = ComputeContext::new(&f2, &values);
        let r0 = s0.compute(&ctx2).unwrap();
        let r1 = s1.compute(&ctx2).unwrap();

        // sector_best_1 (index 0) should now have best_times[0] = 25000
        assert!((r0 - 25000.0).abs() < 0.01);
        // sector_best_2 (index 1) still -1.0 (sector 1 not yet completed)
        assert!((r1 - (-1.0)).abs() < 0.01);
    }

    #[test]
    fn test_sector_best_ignores_invalid_lap() {
        let (mut s0, _, _) = create_sector_best_items();
        let values = HashMap::new();

        let f1 = make_sector_frame(0, 0, 30000, 1);
        s0.compute(&ComputeContext::new(&f1, &values)).unwrap();

        // Transition 0→1 with invalid lap
        let f2 = make_sector_frame(1, 0, 25000, 1);
        let r = s0.compute(&ComputeContext::new(&f2, &values)).unwrap();
        // best_times[0] should NOT have updated → still -1.0
        assert!((r - (-1.0)).abs() < 0.01);
    }

    #[test]
    fn test_sector_best_keeps_best_time() {
        let (mut s0, _, _) = create_sector_best_items();
        let values = HashMap::new();

        // First lap: sector 0→1, time=25000 (best so far)
        let f1 = make_sector_frame(0, 1, 0, 1);
        s0.compute(&ComputeContext::new(&f1, &values)).unwrap();
        let f2 = make_sector_frame(1, 1, 25000, 1);
        s0.compute(&ComputeContext::new(&f2, &values)).unwrap();

        // Second lap: sector 0→1, time=35000 (slower)
        let f3 = make_sector_frame(0, 1, 0, 2);
        s0.compute(&ComputeContext::new(&f3, &values)).unwrap();
        let f4 = make_sector_frame(1, 1, 35000, 2);
        let r = s0.compute(&ComputeContext::new(&f4, &values)).unwrap();
        // Should still be 25000 (best preserved)
        assert!((r - 25000.0).abs() < 0.01);
    }

    #[test]
    fn test_sector_best_multiple_laps() {
        let (mut s0, _, _) = create_sector_best_items();
        let values = HashMap::new();

        // Lap 1: sector 0→1, sector_time=30000
        let f1 = make_sector_frame(0, 1, 0, 1);
        s0.compute(&ComputeContext::new(&f1, &values)).unwrap();
        let f2 = make_sector_frame(1, 1, 30000, 1);
        let r1 = s0.compute(&ComputeContext::new(&f2, &values)).unwrap();
        assert!((r1 - 30000.0).abs() < 0.01);

        // Lap 2: sector 0→1, sector_time=20000 (better!)
        let f3 = make_sector_frame(0, 1, 0, 2);
        s0.compute(&ComputeContext::new(&f3, &values)).unwrap();
        let f4 = make_sector_frame(1, 1, 20000, 2);
        let r2 = s0.compute(&ComputeContext::new(&f4, &values)).unwrap();
        assert!((r2 - 20000.0).abs() < 0.01);

        // Lap 3: sector 0→1, sector_time=25000 (worse, keep 20000)
        let f5 = make_sector_frame(0, 1, 0, 3);
        s0.compute(&ComputeContext::new(&f5, &values)).unwrap();
        let f6 = make_sector_frame(1, 1, 25000, 3);
        let r3 = s0.compute(&ComputeContext::new(&f6, &values)).unwrap();
        assert!((r3 - 20000.0).abs() < 0.01);
    }
}
