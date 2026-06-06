//! 计算项注册中心
//!
//! 管理实时计算项和批量计算项的注册、注销和执行。
//! 按注册顺序执行计算项，失败项跳过不中断。

use super::context::ReferenceSource;
use super::items::{BatchComputeItem, RealtimeComputeItem};
use super::{ComputeContext, ComputeError, ComputeResult, RealtimeComputeRequest};
use crate::TelemetryFrame;
use std::collections::HashMap;
use std::sync::Arc;

/// 计算项注册中心
///
/// 按注册顺序管理实时计算项（RealtimeComputeItem）和批量计算项（BatchComputeItem）。
/// 执行时按注册顺序依次计算，结果存入 `HashMap<String, f64>`。
pub struct ComputeRegistry {
    realtime_items: Vec<Box<dyn RealtimeComputeItem>>,
    batch_items: Vec<Box<dyn BatchComputeItem>>,
    /// 已缓存的参考圈数据
    reference_cache: HashMap<ReferenceSource, Arc<Vec<TelemetryFrame>>>,
}

impl Default for ComputeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputeRegistry {
    /// 创建空的注册中心
    pub fn new() -> Self {
        Self {
            realtime_items: Vec::new(),
            batch_items: Vec::new(),
            reference_cache: HashMap::new(),
        }
    }

    /// 注册实时计算项
    pub fn register_realtime(&mut self, item: Box<dyn RealtimeComputeItem>) {
        self.realtime_items.push(item);
    }

    /// 注册批量计算项
    pub fn register_batch(&mut self, item: Box<dyn BatchComputeItem>) {
        self.batch_items.push(item);
    }

    /// 按名称注销计算项（同时查找实时和批量注册表）
    ///
    /// 返回 true 表示找到并移除了该项，false 表示未找到。
    pub fn unregister(&mut self, name: &str) -> bool {
        if let Some(pos) = self.realtime_items.iter().position(|i| i.name() == name) {
            self.realtime_items.remove(pos);
            return true;
        }
        if let Some(pos) = self.batch_items.iter().position(|i| i.name() == name) {
            self.batch_items.remove(pos);
            return true;
        }
        false
    }

    /// 执行所有已注册的实时计算项
    ///
    /// 按注册顺序依次执行。如果某个计算项失败，跳过该项，继续执行后续项。
    /// 返回成功计算的结果（item_name → value）。
    /// 按名称执行单个实时计算项
    ///
    /// 只执行指定名称的计算项，传入请求中提供的上下文。
    /// 调用方负责构建 `RealtimeComputeRequest`，显式传入所有必要数据。
    pub fn compute_realtime(
        &mut self,
        item_name: &str,
        request: &RealtimeComputeRequest<'_>,
    ) -> ComputeResult<f64> {
        let item = self
            .realtime_items
            .iter_mut()
            .find(|i| i.name() == item_name)
            .ok_or_else(|| ComputeError::ItemNotFound(item_name.to_string()))?;

        let ctx = ComputeContext {
            current_frame: request.current_frame,
            computed_values: request.computed_values,
            reference_lap: request.reference_lap,
            reference_source: request.reference_source.clone(),
        };

        item.compute(&ctx)
    }

    /// 解析参考圈数据（自动加载，方案B）
    ///
    /// 如果缓存命中，返回共享引用（零拷贝）。
    /// 如果缓存未命中，自动从 `.acctlm` 文件加载指定圈号的数据并缓存。
    /// 返回 `Arc` 使得调用方可以在释放 registry 可变借用后使用数据。
    pub fn resolve_reference_lap(
        &mut self,
        source: &ReferenceSource,
    ) -> ComputeResult<Arc<Vec<TelemetryFrame>>> {
        if let Some(arc) = self.reference_cache.get(source) {
            return Ok(Arc::clone(arc));
        }

        let frames = load_reference_lap_from_file(source)?;
        let arc = Arc::new(frames);
        self.reference_cache.insert(source.clone(), Arc::clone(&arc));
        Ok(arc)
    }

    /// 执行指定名称的批量计算项
    pub fn compute_batch(
        &self,
        name: &str,
        current_lap: &[TelemetryFrame],
        reference_lap: &[TelemetryFrame],
    ) -> ComputeResult<Vec<f64>> {
        let item = self
            .batch_items
            .iter()
            .find(|i| i.name() == name)
            .ok_or_else(|| ComputeError::ItemNotFound(name.to_string()))?;

        item.compute_batch(current_lap, reference_lap)
    }

    /// 缓存参考圈数据
    pub fn cache_reference_lap(&mut self, source: ReferenceSource, frames: Vec<TelemetryFrame>) {
        self.reference_cache.insert(source, Arc::new(frames));
    }

    /// 获取已缓存的参考圈数据
    pub fn get_reference_lap(&self, source: &ReferenceSource) -> Option<&[TelemetryFrame]> {
        self.reference_cache.get(source).map(|v| v.as_slice())
    }

    /// 获取实时计算项数量
    pub fn realtime_count(&self) -> usize {
        self.realtime_items.len()
    }

    /// 获取批量计算项数量
    pub fn batch_count(&self) -> usize {
        self.batch_items.len()
    }

    /// 检查指定名称的计算项是否已注册
    pub fn is_registered(&self, name: &str) -> bool {
        self.realtime_items.iter().any(|i| i.name() == name)
            || self.batch_items.iter().any(|i| i.name() == name)
    }
}

/// 从 `.acctlm` 文件自动加载指定圈号的参考帧
fn load_reference_lap_from_file(source: &ReferenceSource) -> ComputeResult<Vec<TelemetryFrame>> {
    use crate::BinaryTelemetryReader;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, TimingSample, TyreSample,
    };

    let reader = BinaryTelemetryReader::open(&source.file_path)?;
    let lap_entries = reader.lap_index();
    let session = reader.read_all_session()?;
    if session.is_empty() {
        return Err(ComputeError::InvalidReferenceData);
    }

    // Find lap boundaries: start_tick and end_tick
    let (start_tick, end_tick) = if let Some(entry) = lap_entries
        .iter()
        .find(|entry| entry.lap_number == source.lap_number)
    {
        (entry.start_tick, entry.end_tick)
    } else {
        // Fallback: detect lap crossings from normalized_car_position
        let mut crossings: Vec<usize> = Vec::new();
        for i in 1..session.len() {
            if session[i - 1].normalized_car_position > 0.8
                && session[i].normalized_car_position < 0.2
            {
                crossings.push(i);
            }
        }
        let lap = source.lap_number as usize;
        if lap > crossings.len() {
            return Err(ComputeError::InvalidReferenceData);
        }
        let start_idx = if lap == 0 { 0 } else { crossings[lap - 1] };
        let end_idx = if lap < crossings.len() {
            crossings[lap] - 1
        } else {
            session.len() - 1
        };
        (session[start_idx].sample_tick, session[end_idx].sample_tick)
    };

    // Read all clusters
    let controls: Vec<ControlSample> = reader.read_all_controls().unwrap_or_default();
    let motion: Vec<MotionSample> = reader.read_all_motion().unwrap_or_default();
    let tyres: Vec<TyreSample> = reader.read_all_tyres().unwrap_or_default();
    let powertrain: Vec<PowertrainSample> = reader.read_all_powertrain().unwrap_or_default();
    let timing: Vec<TimingSample> = reader.read_all_timing()?;
    let car_state: Vec<CarStateSample> = reader.read_all_car_state().unwrap_or_default();
    let environment: Vec<EnvironmentSample> = reader.read_all_environment().unwrap_or_default();
    let other_cars: Vec<OtherCarsSample> = reader.read_all_other_cars().unwrap_or_default();

    // Build tick → index maps (session is the primary cluster for iteration)
    let controls_by_tick: HashMap<u64, ControlSample> =
        controls.into_iter().map(|s| (s.sample_tick, s)).collect();
    let motion_by_tick: HashMap<u64, MotionSample> =
        motion.into_iter().map(|s| (s.sample_tick, s)).collect();
    let tyres_by_tick: HashMap<u64, TyreSample> =
        tyres.into_iter().map(|s| (s.sample_tick, s)).collect();
    let powertrain_by_tick: HashMap<u64, PowertrainSample> =
        powertrain.into_iter().map(|s| (s.sample_tick, s)).collect();
    let timing_by_tick: HashMap<u64, TimingSample> =
        timing.into_iter().map(|s| (s.sample_tick, s)).collect();
    let car_state_by_tick: HashMap<u64, CarStateSample> =
        car_state.into_iter().map(|s| (s.sample_tick, s)).collect();
    let environment_by_tick: HashMap<u64, EnvironmentSample> =
        environment.into_iter().map(|s| (s.sample_tick, s)).collect();
    let other_cars_by_tick: HashMap<u64, OtherCarsSample> =
        other_cars.into_iter().map(|s| (s.sample_tick, s)).collect();

    // Assemble frames: iterate session (primary), fill in other clusters by tick
    let mut frames = Vec::new();
    for s in &session {
        if s.sample_tick < start_tick || s.sample_tick > end_tick {
            continue;
        }
        let tick = s.sample_tick;
        let ts = s.timestamp_ns;
        frames.push(TelemetryFrame {
            sample_tick: tick,
            timestamp_ns: ts,
            controls: controls_by_tick.get(&tick).cloned().unwrap_or_default(),
            motion: motion_by_tick.get(&tick).cloned().unwrap_or_default(),
            tyres: tyres_by_tick.get(&tick).cloned().unwrap_or_default(),
            powertrain: powertrain_by_tick.get(&tick).cloned().unwrap_or_default(),
            session: s.clone(),
            timing: timing_by_tick.get(&tick).cloned().unwrap_or_default(),
            car_state: car_state_by_tick.get(&tick).cloned().unwrap_or_default(),
            environment: environment_by_tick.get(&tick).cloned().unwrap_or_default(),
            other_cars: other_cars_by_tick.get(&tick).cloned().unwrap_or_default(),
        });
    }

    if frames.is_empty() {
        return Err(ComputeError::InvalidReferenceData);
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    };
    use std::collections::HashMap;

    fn make_frame(speed: f32) -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: 0,
            timestamp_ns: 0,
            controls: ControlSample { speed_kmh: speed, ..ControlSample::default() },
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

    fn make_request<'a>(
        frame: &'a TelemetryFrame,
        computed_values: &'a HashMap<String, f64>,
    ) -> RealtimeComputeRequest<'a> {
        RealtimeComputeRequest {
            current_frame: frame,
            computed_values,
            reference_lap: None,
            reference_source: None,
        }
    }

    struct TestItem {
        name: &'static str,
        value: f64,
    }

    impl RealtimeComputeItem for TestItem {
        fn name(&self) -> &str {
            self.name
        }

        fn compute(&mut self, _ctx: &ComputeContext) -> ComputeResult<f64> {
            Ok(self.value)
        }
    }

    struct FailingItem;

    impl RealtimeComputeItem for FailingItem {
        fn name(&self) -> &str {
            "failing"
        }

        fn compute(&mut self, _ctx: &ComputeContext) -> ComputeResult<f64> {
            Err(ComputeError::ComputationFailed("test failure".into()))
        }
    }

    struct ContextAwareItem;

    impl RealtimeComputeItem for ContextAwareItem {
        fn name(&self) -> &str {
            "context_aware"
        }

        fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
            // Return the sum of previously computed values + current speed
            let prev_sum: f64 = ctx.computed_values.values().sum();
            Ok(ctx.current_frame.controls.speed_kmh as f64 + prev_sum)
        }
    }

    #[test]
    fn test_register_and_unregister() {
        let mut registry = ComputeRegistry::new();
        assert_eq!(registry.realtime_count(), 0);

        registry.register_realtime(Box::new(TestItem { name: "a", value: 1.0 }));
        assert_eq!(registry.realtime_count(), 1);
        assert!(registry.is_registered("a"));

        assert!(registry.unregister("a"));
        assert_eq!(registry.realtime_count(), 0);
        assert!(!registry.is_registered("a"));

        // Unregister non-existent returns false
        assert!(!registry.unregister("nonexistent"));
    }

    #[test]
    fn test_compute_realtime_executes_only_requested_item() {
        let mut registry = ComputeRegistry::new();
        registry.register_realtime(Box::new(FailingItem));
        registry.register_realtime(Box::new(TestItem { name: "second", value: 2.0 }));

        let frame = make_frame(100.0);
        let computed_values = HashMap::new();
        let request = make_request(&frame, &computed_values);
        let result = registry.compute_realtime("second", &request).unwrap();

        assert_eq!(result, 2.0);
    }

    #[test]
    fn test_failing_item_returns_error_when_requested() {
        let mut registry = ComputeRegistry::new();
        registry.register_realtime(Box::new(FailingItem));
        registry.register_realtime(Box::new(TestItem { name: "ok", value: 42.0 }));

        let frame = make_frame(100.0);
        let computed_values = HashMap::new();
        let request = make_request(&frame, &computed_values);
        let result = registry.compute_realtime("failing", &request);

        assert!(result.is_err());
    }

    #[test]
    fn test_context_aware_computation() {
        let mut registry = ComputeRegistry::new();
        registry.register_realtime(Box::new(TestItem { name: "a", value: 10.0 }));
        registry.register_realtime(Box::new(ContextAwareItem));

        let frame = make_frame(100.0);
        let mut computed_values = HashMap::new();
        computed_values.insert("a".to_string(), 10.0);
        let request = make_request(&frame, &computed_values);
        let result = registry.compute_realtime("context_aware", &request).unwrap();

        // context_aware should see "a"=10.0, so 100 + 10 = 110
        assert_eq!(result, 110.0);
    }

    #[test]
    fn test_compute_batch_not_found() {
        let registry = ComputeRegistry::new();
        let frames = vec![make_frame(100.0)];
        let result = registry.compute_batch("nonexistent", &frames, &frames);
        assert!(result.is_err());
    }

    struct TestBatchItem;

    impl BatchComputeItem for TestBatchItem {
        fn name(&self) -> &str {
            "test_batch"
        }

        fn compute_batch(
            &self,
            current_lap: &[TelemetryFrame],
            reference_lap: &[TelemetryFrame],
        ) -> ComputeResult<Vec<f64>> {
            Ok(current_lap
                .iter()
                .zip(reference_lap.iter())
                .map(|(c, r)| (c.controls.speed_kmh - r.controls.speed_kmh) as f64)
                .collect())
        }
    }

    #[test]
    fn test_compute_batch_success() {
        let mut registry = ComputeRegistry::new();
        registry.register_batch(Box::new(TestBatchItem));

        let current = vec![make_frame(150.0), make_frame(160.0)];
        let reference = vec![make_frame(100.0), make_frame(100.0)];
        let result = registry.compute_batch("test_batch", &current, &reference).unwrap();

        assert_eq!(result, vec![50.0, 60.0]);
    }

    #[test]
    fn test_reference_cache() {
        let mut registry = ComputeRegistry::new();
        let source = ReferenceSource {
            file_path: std::path::PathBuf::from("test.acctlm"),
            lap_number: 3,
        };
        let frames = vec![make_frame(100.0)];
        registry.cache_reference_lap(source.clone(), frames);

        let cached = registry.get_reference_lap(&source).unwrap();
        assert_eq!(cached.len(), 1);
    }
}
