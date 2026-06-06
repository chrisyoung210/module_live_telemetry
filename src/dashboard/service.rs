//! Dashboard 数据服务
//!
//! 管理计算项订阅、按频率调度计算、聚合结果并通过 DataSink 回传。

use crate::compute::{ComputeError, ComputeRegistry};
use crate::compute::context::ReferenceSource;
use crate::dashboard::sink::DataSink;
use crate::TelemetryFrame;
use crossbeam_channel::Receiver;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Dashboard 数据服务
///
/// 接收上游订阅请求，按每个 item 的独立频率调度计算，
/// 聚合结果并通过 DataSink 回传给上游程序。
///
/// # 使用示例
///
/// ```no_run
/// use module_live_telemetry::compute::ComputeRegistry;
/// use module_live_telemetry::dashboard::service::DashboardService;
/// use module_live_telemetry::dashboard::sink::ChannelSink;
/// use std::time::Duration;
/// use crossbeam_channel::bounded;
///
/// let registry = ComputeRegistry::new();
/// let (tx, rx) = bounded(10);
/// let sink = ChannelSink::new(tx);
/// let mut service = DashboardService::new(registry, Box::new(sink));
/// service.subscribe("speed_mps".into(), Duration::from_millis(50), None);
/// ```
pub struct DashboardService {
    registry: ComputeRegistry,
    sink: Box<dyn DataSink>,
    /// item_name → 计算间隔
    subscriptions: HashMap<String, Duration>,
    /// item_name → 下次计算时间
    next_schedule: HashMap<String, Instant>,
    /// item_name → 参考圈数据来源（None 表示不需要参考圈）
    reference_sources: HashMap<String, Option<ReferenceSource>>,
    /// 是否已经报告过 sink 发送错误（每个 service 实例只报告一次）
    sink_error_reported: bool,
}

impl DashboardService {
    /// 创建新的 Dashboard 服务
    pub fn new(registry: ComputeRegistry, sink: Box<dyn DataSink>) -> Self {
        Self {
            registry,
            sink,
            subscriptions: HashMap::new(),
            next_schedule: HashMap::new(),
            reference_sources: HashMap::new(),
            sink_error_reported: false,
        }
    }

    /// 订阅计算项
    ///
    /// `item_name` 必须已在 ComputeRegistry 中注册，否则返回 `ComputeError::ItemNotFound`。
    /// `interval` 指定该 item 的计算频率。
    /// `reference_source` 为动态计算项提供参考圈数据来源（静态计算项可传 `None`）。
    pub fn subscribe(
        &mut self,
        item_name: String,
        interval: Duration,
        reference_source: Option<ReferenceSource>,
    ) -> crate::compute::ComputeResult<()> {
        if !self.registry.is_registered(&item_name) {
            return Err(ComputeError::ItemNotFound(item_name));
        }
        self.subscriptions.insert(item_name.clone(), interval);
        self.next_schedule
            .insert(item_name.clone(), Instant::now() + interval);
        self.reference_sources
            .insert(item_name, reference_source);
        Ok(())
    }

    /// 取消订阅
    pub fn unsubscribe(&mut self, item_name: &str) {
        self.subscriptions.remove(item_name);
        self.next_schedule.remove(item_name);
        self.reference_sources.remove(item_name);
    }

    /// 获取当前订阅数
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// 检查 item 是否已订阅
    pub fn is_subscribed(&self, item_name: &str) -> bool {
        self.subscriptions.contains_key(item_name)
    }

    /// 主运行循环
    ///
    /// 从 `receiver` 接收遥测帧。每收到一帧，检查哪些订阅的 item 到了计算时间，
    /// 为每个到期项独立构建计算上下文（包括参考圈），计算结果并通过 sink 回传。
    ///
    /// 当 `receiver` 断开（发送端被 drop）时，循环自动退出。
    pub fn run(&mut self, receiver: Receiver<Arc<TelemetryFrame>>) {
        use crate::compute::RealtimeComputeRequest;

        for frame_arc in receiver {
            let now = Instant::now();
            let frame = &*frame_arc;

            // 如果没有任何订阅，跳过计算
            if self.subscriptions.is_empty() {
                continue;
            }

            // 收集本轮需要计算的 items
            let mut items_to_compute: Vec<String> = Vec::new();
            for (name, next_time) in &self.next_schedule {
                if now >= *next_time {
                    items_to_compute.push(name.clone());
                }
            }

            if items_to_compute.is_empty() {
                continue;
            }

            // 逐项计算：每个 item 获得独立的上下文（包括参考圈）
            let mut sparse_result = HashMap::new();
            for name in &items_to_compute {
                // 解析参考圈（如果该 item 订阅时携带了 ReferenceSource）
                // 使用 Arc 避免引用 self.registry 跨越 compute_realtime_item 的可变借用
                let reference_arc = if let Some(Some(ref source)) = self.reference_sources.get(name) {
                    match self.registry.resolve_reference_lap(source) {
                        Ok(arc) => Some(arc),
                        Err(err) => {
                            eprintln!("dashboard: failed to load reference lap for '{name}': {err}");
                            None
                        }
                    }
                } else {
                    None
                };

                let reference_lap = reference_arc.as_ref().map(|arc| arc.as_slice());

                let request = RealtimeComputeRequest {
                    current_frame: frame,
                    computed_values: &sparse_result,
                    reference_lap,
                    reference_source: self
                        .reference_sources
                        .get(name)
                        .and_then(|r| r.clone()),
                };

                match self.registry.compute_realtime(name, &request) {
                    Ok(value) => {
                        sparse_result.insert(name.clone(), value);
                        // 基于上一次计划时间推进，避免帧处理耗时导致的累积漂移
                        if let Some(interval) = self.subscriptions.get(name) {
                            let prev = self.next_schedule.get(name).copied().unwrap_or(now);
                            self.next_schedule.insert(name.clone(), prev + *interval);
                        }
                    }
                    Err(err) => {
                        eprintln!("dashboard: compute item '{name}' failed: {err}");
                        // 失败时不推进 schedule，下次帧到达时立即重试
                    }
                }
            }

            // 通过 sink 回传
            if !sparse_result.is_empty() {
                if let Err(err) = self.sink.send(sparse_result) {
                    if !self.sink_error_reported {
                        eprintln!("dashboard sink send failed: {err}; subsequent errors suppressed");
                        self.sink_error_reported = true;
                    }
                }
            }
        }
    }

    /// 获取注册中心的不可变引用
    pub fn registry(&self) -> &ComputeRegistry {
        &self.registry
    }

    /// 获取注册中心的可变引用
    pub fn registry_mut(&mut self) -> &mut ComputeRegistry {
        &mut self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::items::SpeedMps;
    use crate::dashboard::sink::ChannelSink;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    };
    use crossbeam_channel::bounded;

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
    fn test_subscribe_and_unsubscribe() {
        let mut reg = ComputeRegistry::new();
        reg.register_realtime(Box::new(SpeedMps));

        let (tx, _rx) = bounded::<HashMap<String, f64>>(10);
        let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

        assert_eq!(service.subscription_count(), 0);

        service.subscribe("speed_mps".into(), Duration::from_millis(100), None).unwrap();
        assert!(service.is_subscribed("speed_mps"));
        assert_eq!(service.subscription_count(), 1);

        service.unsubscribe("speed_mps");
        assert!(!service.is_subscribed("speed_mps"));
        assert_eq!(service.subscription_count(), 0);
    }

    #[test]
    fn test_subscribe_unknown_item_returns_error() {
        let reg = ComputeRegistry::new();
        let (tx, _rx) = bounded::<HashMap<String, f64>>(10);
        let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

        let result = service.subscribe("nonexistent".into(), Duration::from_millis(100), None);
        assert!(result.is_err());
        assert_eq!(service.subscription_count(), 0);
    }

    #[test]
    fn test_empty_subscriptions_skip_computation() {
        let reg = ComputeRegistry::new();
        let (tx, rx) = bounded::<HashMap<String, f64>>(10);
        let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(10);
        let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

        // Send a frame, but no subscriptions — nothing should be computed
        frame_tx.send(Arc::new(make_frame(100.0))).unwrap();
        drop(frame_tx); // Close to stop the loop

        service.run(frame_rx);

        // No data should be sent
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_data_flow_with_subscription() {
        let mut reg = ComputeRegistry::new();
        reg.register_realtime(Box::new(SpeedMps));

        let (data_tx, data_rx) = bounded::<HashMap<String, f64>>(10);
        let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(10);
        let mut service =
            DashboardService::new(reg, Box::new(ChannelSink::new(data_tx)));

        // Subscribe with a very short interval so it triggers on the first frame
        service.subscribe("speed_mps".into(), Duration::from_nanos(1), None).unwrap();

        // Send a frame
        frame_tx.send(Arc::new(make_frame(100.0))).unwrap();
        drop(frame_tx);

        service.run(frame_rx);

        // Should receive data
        let result = data_rx.try_recv().unwrap();
        assert!((result.get("speed_mps").copied().unwrap_or(0.0) - 27.7777).abs() < 0.1);
    }
}
