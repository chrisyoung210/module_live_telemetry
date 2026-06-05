//! Dashboard 数据服务
//!
//! 管理计算项订阅、按频率调度计算、聚合结果并通过 DataSink 回传。

use crate::compute::ComputeRegistry;
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
/// service.subscribe("speed_mps".into(), Duration::from_millis(50));
/// ```
pub struct DashboardService {
    registry: ComputeRegistry,
    sink: Box<dyn DataSink>,
    /// item_name → 计算间隔
    subscriptions: HashMap<String, Duration>,
    /// item_name → 下次计算时间
    next_schedule: HashMap<String, Instant>,
}

impl DashboardService {
    /// 创建新的 Dashboard 服务
    pub fn new(registry: ComputeRegistry, sink: Box<dyn DataSink>) -> Self {
        Self {
            registry,
            sink,
            subscriptions: HashMap::new(),
            next_schedule: HashMap::new(),
        }
    }

    /// 订阅计算项
    ///
    /// `item_name` 必须已在 ComputeRegistry 中注册。
    /// `interval` 指定该 item 的计算频率。
    pub fn subscribe(&mut self, item_name: String, interval: Duration) {
        self.subscriptions.insert(item_name.clone(), interval);
        self.next_schedule
            .insert(item_name, Instant::now() + interval);
    }

    /// 取消订阅
    pub fn unsubscribe(&mut self, item_name: &str) {
        self.subscriptions.remove(item_name);
        self.next_schedule.remove(item_name);
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
    /// 执行这些 item 的计算，聚合结果并通过 sink 回传。
    ///
    /// 当 `receiver` 断开（发送端被 drop）时，循环自动退出。
    pub fn run(&mut self, receiver: Receiver<Arc<TelemetryFrame>>) {
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

            // 执行所有已注册的实时计算项（但只收集被订阅的）
            let all_results = self.registry.compute_realtime(frame);

            // 过滤：只保留被订阅且本轮到期的 items
            let mut sparse_result = HashMap::new();
            for name in &items_to_compute {
                if let Some(value) = all_results.get(name) {
                    sparse_result.insert(name.clone(), *value);
                }

                // 更新下次计算时间
                if let Some(interval) = self.subscriptions.get(name) {
                    self.next_schedule.insert(name.clone(), now + *interval);
                }
            }

            // 通过 sink 回传
            if !sparse_result.is_empty() {
                self.sink.send(sparse_result);
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

        service.subscribe("speed_mps".into(), Duration::from_millis(100));
        assert!(service.is_subscribed("speed_mps"));
        assert_eq!(service.subscription_count(), 1);

        service.unsubscribe("speed_mps");
        assert!(!service.is_subscribed("speed_mps"));
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
        service.subscribe("speed_mps".into(), Duration::from_nanos(1));

        // Send a frame
        frame_tx.send(Arc::new(make_frame(100.0))).unwrap();
        drop(frame_tx);

        service.run(frame_rx);

        // Should receive data
        let result = data_rx.try_recv().unwrap();
        assert!((result.get("speed_mps").copied().unwrap_or(0.0) - 27.7777).abs() < 0.1);
    }
}
