//! 计算项注册中心
//!
//! 管理 calculated item（RealtimeComputeItem 和 BatchComputeItem）的注册、注销和执行。
//! Raw item 不由本模块管理——它们从 TelemetryFrame 字段自动解析。

use super::context::ReferenceSource;
use super::items::{BatchComputeItem, RealtimeComputeItem};
use super::{ComputeContext, ComputeError, ComputeResult, RealtimeComputeRequest};
use crate::TelemetryFrame;
use std::collections::HashMap;
use std::sync::Arc;

/// 参考圈缓存最大条目数
const MAX_CACHE_ENTRIES: usize = 4;

/// 计算项注册中心（仅管理 calculated item）
///
/// Raw item 通过 `TelemetryFrame::raw_field_value()` 自动解析，无需注册。
/// 本注册中心只存放需要自定义计算逻辑的 calculated item。
pub struct ComputeRegistry {
    /// calculated 实时计算项（按名称索引）
    realtime_calc: HashMap<String, Box<dyn RealtimeComputeItem>>,
    /// calculated 批量计算项（按名称索引）
    batch_calc: HashMap<String, Box<dyn BatchComputeItem>>,
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
            realtime_calc: HashMap::new(),
            batch_calc: HashMap::new(),
            reference_cache: HashMap::new(),
        }
    }

    /// Create a registry with all builtin realtime dashboard items registered.
    pub fn with_builtin_dashboard_items() -> ComputeResult<Self> {
        let mut registry = Self::new();
        registry.register_builtin_dashboard_items()?;
        Ok(registry)
    }

    /// Register all builtin realtime dashboard items.
    pub fn register_builtin_dashboard_items(&mut self) -> ComputeResult<()> {
        use super::items::{
            create_prev_sector_items, create_sector_best_items, CarCoordX, CarCoordZ,
            DeltaTimeToLifeBestLap, DeltaTimeToLifeBestLapInterpolated, DeltaTimeToSessionBestLap,
            // DeltaTimeToSessionBestCenterline, DeltaTimeToSessionBestFast,
            DeltaTimeToSessionBestLapInterpolated,
            // DeltaTimeToSessionBestRefPoly,
            LapDistance, PredictLapTimeByLifeBestLap, PredictLapTimeBySessionBestLap,
        };

        self.register_calc_realtime(Box::new(DeltaTimeToLifeBestLap::new()))?;
        self.register_calc_realtime(Box::new(DeltaTimeToLifeBestLapInterpolated::new()))?;
        self.register_calc_realtime(Box::new(DeltaTimeToSessionBestLap::new()))?;
        self.register_calc_realtime(Box::new(DeltaTimeToSessionBestLapInterpolated::new()))?;
        // TODO: 暂时隐藏 RefPoly/Centerline/Fast，待定位 10~20ms 偏差后重新启用
        // self.register_calc_realtime(Box::new(DeltaTimeToSessionBestRefPoly::new()))?;
        // self.register_calc_realtime(Box::new(DeltaTimeToSessionBestCenterline::new()))?;
        // self.register_calc_realtime(Box::new(DeltaTimeToSessionBestFast::new()))?;

        self.register_calc_realtime(Box::new(PredictLapTimeByLifeBestLap::new()))?;
        self.register_calc_realtime(Box::new(PredictLapTimeBySessionBestLap::new()))?;

        self.register_calc_realtime(Box::new(CarCoordX::new()))?;
        self.register_calc_realtime(Box::new(CarCoordZ::new()))?;
        self.register_calc_realtime(Box::new(LapDistance::new()))?;

        let (prev_sector_time, prev_sector_number) = create_prev_sector_items();
        self.register_calc_realtime(Box::new(prev_sector_time))?;
        self.register_calc_realtime(Box::new(prev_sector_number))?;

        let (sector_best_1, sector_best_2, sector_best_3) = create_sector_best_items();
        self.register_calc_realtime(Box::new(sector_best_1))?;
        self.register_calc_realtime(Box::new(sector_best_2))?;
        self.register_calc_realtime(Box::new(sector_best_3))?;

        Ok(())
    }

    /// 注册 calculated 实时计算项
    ///
    /// # 校验规则
    ///
    /// - 名称不能为空
    /// - 不能与已注册的 realtime 或 batch 计算项重名
    ///
    /// # 错误
    ///
    /// 返回 `ComputeError::InvalidRegistration`。
    pub fn register_calc_realtime(
        &mut self,
        item: Box<dyn RealtimeComputeItem>,
    ) -> ComputeResult<()> {
        let name = item.name();
        if name.is_empty() {
            return Err(ComputeError::InvalidRegistration(
                "计算项名称不能为空".into(),
            ));
        }
        let name = name.to_string();
        if self.realtime_calc.contains_key(&name) || self.batch_calc.contains_key(&name) {
            return Err(ComputeError::InvalidRegistration(format!(
                "计算项 '{}' 已注册",
                name
            )));
        }
        self.realtime_calc.insert(name, item);
        Ok(())
    }

    /// 注册 calculated 批量计算项
    ///
    /// 校验规则同 [`register_calc_realtime`]。
    pub fn register_calc_batch(&mut self, item: Box<dyn BatchComputeItem>) -> ComputeResult<()> {
        let name = item.name();
        if name.is_empty() {
            return Err(ComputeError::InvalidRegistration(
                "计算项名称不能为空".into(),
            ));
        }
        let name = name.to_string();
        if self.realtime_calc.contains_key(&name) || self.batch_calc.contains_key(&name) {
            return Err(ComputeError::InvalidRegistration(format!(
                "计算项 '{}' 已注册",
                name
            )));
        }
        self.batch_calc.insert(name, item);
        Ok(())
    }

    /// 按名称注销 calculated 计算项（同时查找实时和批量表）
    ///
    /// 返回 true 表示找到并移除了该项，false 表示未找到。
    pub fn unregister(&mut self, name: &str) -> bool {
        self.realtime_calc.remove(name).is_some() || self.batch_calc.remove(name).is_some()
    }

    /// 按名称执行单个 calculated 实时计算项
    ///
    /// 只执行 specified 名称的计算项，传入请求中提供的上下文。
    /// 调用方负责构建 `RealtimeComputeRequest`，显式传入所有必要数据。
    pub fn compute_realtime(
        &mut self,
        item_name: &str,
        request: &RealtimeComputeRequest<'_>,
    ) -> ComputeResult<f64> {
        let item = self
            .realtime_calc
            .get_mut(item_name)
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
    /// 如果缓存未命中，自动从 `.acctlm2` 文件加载指定圈号的数据并缓存。
    /// 返回 `Arc` 使得调用方可以在释放 registry 可变借用后使用数据。
    pub fn resolve_reference_lap(
        &mut self,
        source: &ReferenceSource,
    ) -> ComputeResult<Arc<Vec<TelemetryFrame>>> {
        if let Some(arc) = self.reference_cache.get(source) {
            return Ok(Arc::clone(arc));
        }

        if source.is_session_best() {
            return Err(ComputeError::NoValidData);
        }

        eprintln!(
            "compute registry: loading reference lap path='{}' lap={}",
            source.file_path.display(),
            source.lap_number
        );
        let frames = load_reference_lap_from_file(source)?;
        let frame_count = frames.len();
        let arc = Arc::new(frames);

        // Evict if full before inserting
        if self.reference_cache.len() >= MAX_CACHE_ENTRIES
            && !self.reference_cache.contains_key(source)
        {
            if let Some(key) = self.reference_cache.keys().next().cloned() {
                self.reference_cache.remove(&key);
            }
        }
        self.reference_cache
            .insert(source.clone(), Arc::clone(&arc));
        eprintln!(
            "compute registry: cached reference lap path='{}' lap={} frames={}",
            source.file_path.display(),
            source.lap_number,
            frame_count
        );
        Ok(arc)
    }

    /// 执行指定名称的 calculated 批量计算项
    pub fn compute_batch(
        &self,
        name: &str,
        current_lap: &[TelemetryFrame],
        reference_lap: &[TelemetryFrame],
    ) -> ComputeResult<Vec<f64>> {
        let item = self
            .batch_calc
            .get(name)
            .ok_or_else(|| ComputeError::ItemNotFound(name.to_string()))?;

        item.compute_batch(current_lap, reference_lap)
    }

    /// 缓存参考圈数据
    ///
    /// 如果缓存已满（max `MAX_CACHE_ENTRIES`），淘汰一个已有条目后插入新数据。
    pub fn cache_reference_lap(&mut self, source: ReferenceSource, frames: Vec<TelemetryFrame>) {
        let frame_count = frames.len();
        if self.reference_cache.len() >= MAX_CACHE_ENTRIES
            && !self.reference_cache.contains_key(&source)
        {
            // 淘汰一个已有条目（HashMap 无序，任意选择一个）
            if let Some(key) = self.reference_cache.keys().next().cloned() {
                self.reference_cache.remove(&key);
            }
        }
        self.reference_cache
            .insert(source.clone(), Arc::new(frames));
        eprintln!(
            "compute registry: cache_reference_lap path='{}' lap={} frames={}",
            source.file_path.display(),
            source.lap_number,
            frame_count
        );
    }

    /// 替换参考圈数据（直接覆盖，不清除其他缓存条目）
    ///
    /// 与 [`cache_reference_lap`] 不同，此方法不会触发 LRU 淘汰。
    /// 适用于运行时用更快圈速替换参考圈的场景。
    pub fn replace_reference(&mut self, source: ReferenceSource, frames: Vec<TelemetryFrame>) {
        let frame_count = frames.len();
        self.reference_cache
            .insert(source.clone(), Arc::new(frames));
        eprintln!(
            "compute registry: replace_reference path='{}' lap={} frames={}",
            source.file_path.display(),
            source.lap_number,
            frame_count
        );
    }

    /// 获取已缓存的参考圈数据
    pub fn get_reference_lap(&self, source: &ReferenceSource) -> Option<&[TelemetryFrame]> {
        self.reference_cache.get(source).map(|v| v.as_slice())
    }

    /// 获取 calculated 实时计算项数量
    pub fn calc_realtime_count(&self) -> usize {
        self.realtime_calc.len()
    }

    /// 获取 calculated 批量计算项数量
    pub fn calc_batch_count(&self) -> usize {
        self.batch_calc.len()
    }

    /// 检查指定名称的 calculated 计算项是否已注册
    pub fn is_registered(&self, name: &str) -> bool {
        self.realtime_calc.contains_key(name) || self.batch_calc.contains_key(name)
    }

    /// 获取所有已注册 calculated 计算项的名称列表
    pub fn registered_item_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .realtime_calc
            .keys()
            .map(|s| s.as_str())
            .chain(self.batch_calc.keys().map(|s| s.as_str()))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// 查询指定 calculated 实时计算项的依赖项名称列表。
    ///
    /// 返回该 item 通过 `RealtimeComputeItem::dependencies()` 声明的依赖，
    /// 调用方（如 DashboardService）可据此在同一帧内对计算项做拓扑排序，
    /// 确保依赖项先于本项计算。
    pub fn dependencies_of(&self, name: &str) -> Vec<String> {
        self.realtime_calc
            .get(name)
            .map(|item| item.dependencies().iter().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }
}

/// 从 `.acctlm2` 文件自动加载指定圈号的参考帧
fn load_reference_lap_from_file(source: &ReferenceSource) -> ComputeResult<Vec<TelemetryFrame>> {
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
        PowertrainSample, TimingSample, TyreSample,
    };
    use crate::BinaryTelemetryReader;

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
    let timing: Vec<TimingSample> = reader.read_all_timing().unwrap_or_default();
    let car_state: Vec<CarStateSample> = reader.read_all_car_state().unwrap_or_default();
    let environment: Vec<EnvironmentSample> = reader.read_all_environment().unwrap_or_default();
    let other_cars: Vec<OtherCarsSample> = reader.read_all_other_cars().unwrap_or_default();

    // Build frames from aligned clusters
    let max_len = [
        controls.len(),
        motion.len(),
        tyres.len(),
        powertrain.len(),
        timing.len(),
        car_state.len(),
        environment.len(),
        other_cars.len(),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);

    let mut frames = Vec::with_capacity(max_len);
    for i in 0..max_len {
        let tick = controls.get(i).map(|c| c.sample_tick).unwrap_or(0);
        if tick < start_tick || tick > end_tick {
            continue;
        }
        frames.push(TelemetryFrame {
            sample_tick: tick,
            timestamp_ns: controls.get(i).map(|c| c.timestamp_ns).unwrap_or(0),
            controls: controls.get(i).copied().unwrap_or_default(),
            motion: motion.get(i).copied().unwrap_or_default(),
            tyres: tyres.get(i).cloned().unwrap_or_default(),
            powertrain: powertrain.get(i).copied().unwrap_or_default(),
            session: session.get(i).cloned().unwrap_or_default(),
            timing: timing.get(i).cloned().unwrap_or_default(),
            car_state: car_state.get(i).cloned().unwrap_or_default(),
            environment: environment.get(i).copied().unwrap_or_default(),
            other_cars: other_cars.get(i).cloned().unwrap_or_default(),
        });
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
        PowertrainSample, SessionSample, TimingSample, TyreSample,
    };

    // ---- Test items ----

    struct TestItem;
    impl RealtimeComputeItem for TestItem {
        fn name(&self) -> &str {
            "test_item"
        }
        fn compute(&mut self, _ctx: &ComputeContext) -> ComputeResult<f64> {
            Ok(42.0)
        }
    }

    #[allow(dead_code)]
    struct TestFailingItem;
    impl RealtimeComputeItem for TestFailingItem {
        fn name(&self) -> &str {
            "failing"
        }
        fn compute(&mut self, _ctx: &ComputeContext) -> ComputeResult<f64> {
            Err(ComputeError::ComputationFailed("test error".into()))
        }
    }

    struct TestBatchItem;
    impl BatchComputeItem for TestBatchItem {
        fn name(&self) -> &str {
            "test_batch"
        }
        fn compute_batch(
            &self,
            current: &[TelemetryFrame],
            reference: &[TelemetryFrame],
        ) -> ComputeResult<Vec<f64>> {
            Ok(current
                .iter()
                .zip(reference.iter())
                .map(|(c, r)| c.controls.speed_kmh as f64 - r.controls.speed_kmh as f64)
                .collect())
        }
    }

    fn make_frame(speed: f32) -> TelemetryFrame {
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
            session: SessionSample::default(),
            timing: TimingSample::default(),
            car_state: CarStateSample::default(),
            environment: EnvironmentSample::default(),
            other_cars: OtherCarsSample::default(),
        }
    }

    #[test]
    fn test_register_and_compute_realtime() {
        let mut registry = ComputeRegistry::new();
        registry.register_calc_realtime(Box::new(TestItem)).unwrap();

        let frame = make_frame(100.0);
        let req = RealtimeComputeRequest {
            current_frame: &frame,
            computed_values: &HashMap::new(),
            reference_lap: None,
            reference_source: None,
        };
        let val = registry.compute_realtime("test_item", &req).unwrap();
        assert!((val - 42.0).abs() < 0.01);
    }

    #[test]
    fn test_unregister() {
        let mut registry = ComputeRegistry::new();
        registry.register_calc_realtime(Box::new(TestItem)).unwrap();
        assert!(registry.unregister("test_item"));
        assert!(!registry.unregister("test_item"));
    }

    #[test]
    fn test_is_registered() {
        let mut registry = ComputeRegistry::new();
        assert!(!registry.is_registered("test_item"));
        registry.register_calc_realtime(Box::new(TestItem)).unwrap();
        assert!(registry.is_registered("test_item"));
    }

    #[test]
    fn test_with_builtin_dashboard_items_registers_catalog_items() {
        let registry = ComputeRegistry::with_builtin_dashboard_items().unwrap();

        for entry in crate::compute::items::all_builtin_calculated_items() {
            assert!(
                registry.is_registered(&entry.key.name),
                "builtin item '{}' should be registered",
                entry.key
            );
        }
    }

    #[test]
    fn test_item_not_found() {
        let mut registry = ComputeRegistry::new();
        let frame = make_frame(100.0);
        let req = RealtimeComputeRequest {
            current_frame: &frame,
            computed_values: &HashMap::new(),
            reference_lap: None,
            reference_source: None,
        };
        assert!(registry.compute_realtime("nonexistent", &req).is_err());
    }

    #[test]
    fn test_compute_batch_success() {
        let mut registry = ComputeRegistry::new();
        registry
            .register_calc_batch(Box::new(TestBatchItem))
            .unwrap();

        let current = vec![make_frame(150.0), make_frame(160.0)];
        let reference = vec![make_frame(100.0), make_frame(100.0)];
        let result = registry
            .compute_batch("test_batch", &current, &reference)
            .unwrap();

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

    #[test]
    fn test_session_best_reference_is_runtime_only() {
        let mut registry = ComputeRegistry::new();
        let source = ReferenceSource::session_best();

        assert_eq!(
            registry.resolve_reference_lap(&source).unwrap_err(),
            ComputeError::NoValidData
        );

        registry.replace_reference(source.clone(), vec![make_frame(100.0), make_frame(110.0)]);
        let cached = registry.resolve_reference_lap(&source).unwrap();
        assert_eq!(cached.len(), 2);
    }

    // ---- Registration validation ----

    struct EmptyNameItem;
    impl RealtimeComputeItem for EmptyNameItem {
        fn name(&self) -> &str {
            ""
        }
        fn compute(&mut self, _ctx: &ComputeContext) -> ComputeResult<f64> {
            Ok(0.0)
        }
    }

    #[test]
    fn test_register_empty_name_rejected() {
        let mut registry = ComputeRegistry::new();
        let result = registry.register_calc_realtime(Box::new(EmptyNameItem));
        assert!(result.is_err());
        match result.unwrap_err() {
            ComputeError::InvalidRegistration(msg) => {
                assert!(msg.contains("不能为空"));
            }
            _ => panic!("expected InvalidRegistration"),
        }
    }

    #[test]
    fn test_register_duplicate_name_rejected() {
        let mut registry = ComputeRegistry::new();
        registry.register_calc_realtime(Box::new(TestItem)).unwrap();
        let result = registry.register_calc_realtime(Box::new(TestItem));
        assert!(result.is_err());
        match result.unwrap_err() {
            ComputeError::InvalidRegistration(msg) => {
                assert!(msg.contains("已注册"));
            }
            _ => panic!("expected InvalidRegistration"),
        }
    }

    #[test]
    fn test_register_batch_duplicate_with_realtime() {
        let mut registry = ComputeRegistry::new();
        registry.register_calc_realtime(Box::new(TestItem)).unwrap();
        // Try to register a batch item with the same name
        struct ConflictingBatch;
        impl BatchComputeItem for ConflictingBatch {
            fn name(&self) -> &str {
                "test_item"
            }
            fn compute_batch(
                &self,
                _c: &[TelemetryFrame],
                _r: &[TelemetryFrame],
            ) -> ComputeResult<Vec<f64>> {
                Ok(vec![])
            }
        }
        let result = registry.register_calc_batch(Box::new(ConflictingBatch));
        assert!(result.is_err());
    }
}
