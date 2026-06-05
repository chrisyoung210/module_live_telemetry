//! 计算项注册中心
//!
//! 管理实时计算项和批量计算项的注册、注销和执行。
//! 按注册顺序执行计算项，失败项跳过不中断。

use super::context::ReferenceSource;
use super::items::{BatchComputeItem, RealtimeComputeItem};
use super::{ComputeContext, ComputeError, ComputeResult};
use crate::TelemetryFrame;
use std::collections::HashMap;

/// 计算项注册中心
///
/// 按注册顺序管理实时计算项（RealtimeComputeItem）和批量计算项（BatchComputeItem）。
/// 执行时按注册顺序依次计算，结果存入 `HashMap<String, f64>`。
pub struct ComputeRegistry {
    realtime_items: Vec<Box<dyn RealtimeComputeItem>>,
    batch_items: Vec<Box<dyn BatchComputeItem>>,
    /// 已缓存的参考圈数据
    reference_cache: HashMap<ReferenceSource, Vec<TelemetryFrame>>,
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
    pub fn compute_realtime(&mut self, frame: &TelemetryFrame) -> HashMap<String, f64> {
        let mut computed_values = HashMap::new();

        for item in &mut self.realtime_items {
            let ctx = ComputeContext {
                current_frame: frame,
                computed_values: &computed_values,
                reference_lap: None,
                reference_source: None,
            };

            match item.compute(&ctx) {
                Ok(value) => {
                    computed_values.insert(item.name().to_string(), value);
                }
                Err(err) => {
                    eprintln!(
                        "compute item '{}' failed: {err}; skipping",
                        item.name()
                    );
                }
            }
        }

        computed_values
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
        self.reference_cache.insert(source, frames);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    };

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
    fn test_compute_realtime_order() {
        let mut registry = ComputeRegistry::new();
        registry.register_realtime(Box::new(TestItem { name: "first", value: 1.0 }));
        registry.register_realtime(Box::new(TestItem { name: "second", value: 2.0 }));

        let frame = make_frame(100.0);
        let results = registry.compute_realtime(&frame);

        assert_eq!(results.get("first"), Some(&1.0));
        assert_eq!(results.get("second"), Some(&2.0));
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_failing_item_skipped() {
        let mut registry = ComputeRegistry::new();
        registry.register_realtime(Box::new(FailingItem));
        registry.register_realtime(Box::new(TestItem { name: "ok", value: 42.0 }));

        let frame = make_frame(100.0);
        let results = registry.compute_realtime(&frame);

        // Failing item should not appear in results
        assert!(!results.contains_key("failing"));
        // OK item should still be present
        assert_eq!(results.get("ok"), Some(&42.0));
    }

    #[test]
    fn test_context_aware_computation() {
        let mut registry = ComputeRegistry::new();
        registry.register_realtime(Box::new(TestItem { name: "a", value: 10.0 }));
        registry.register_realtime(Box::new(ContextAwareItem));

        let frame = make_frame(100.0);
        let results = registry.compute_realtime(&frame);

        // context_aware should see "a"=10.0, so 100 + 10 = 110
        assert_eq!(results.get("context_aware"), Some(&110.0));
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
