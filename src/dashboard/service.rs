//! Dashboard 数据服务
//!
//! 管理计算项订阅、按频率调度计算、聚合结果并通过 DataSink 回传。
//!
//! 支持三种 item 类型：
//! - `raw:*` — 从 TelemetryFrame 字段直接读取，无需注册
//! - `calc:*` — 通过 ComputeRegistry 计算
//! - `system:*` — 系统信息（未来）

use crate::compute::{ComputeError, ComputeRegistry, RealtimeComputeRequest};
use crate::compute::context::ReferenceSource;
use crate::dashboard::sink::DataSink;
use crate::item_key::{ItemKey, ItemType};
use crate::TelemetryFrame;
use crossbeam_channel::{Receiver, Select};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 运行时动态修改 dashboard 订阅的命令
pub enum DashboardCommand {
    Subscribe {
        item_key: ItemKey,
        interval: Duration,
        reference_source: Option<ReferenceSource>,
    },
    Unsubscribe(ItemKey),
    /// 原子替换全部订阅
    ReplaceAll {
        items: Vec<(ItemKey, Duration, Option<ReferenceSource>)>,
    },
}

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
/// use module_live_telemetry::item_key::ItemKey;
/// use std::time::Duration;
/// use crossbeam_channel::bounded;
///
/// let registry = ComputeRegistry::new();
/// let (tx, rx) = bounded(10);
/// let sink = ChannelSink::new(tx);
/// let mut service = DashboardService::new(registry, Box::new(sink));
/// service.subscribe(
///     ItemKey::parse("raw:controls.speed_kmh").unwrap(),
///     Duration::from_millis(50),
///     None,
/// ).unwrap();
/// ```
pub struct DashboardService {
    registry: ComputeRegistry,
    sink: Box<dyn DataSink>,
    /// item_key → 计算间隔
    subscriptions: HashMap<ItemKey, Duration>,
    /// item_key → 下次计算时间
    next_schedule: HashMap<ItemKey, Instant>,
    /// item_key → 参考圈数据来源（None 表示不需要参考圈）
    reference_sources: HashMap<ItemKey, Option<ReferenceSource>>,
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
    /// - `raw:*` items are auto-validated via `TelemetryFrame::is_raw_field()`
    /// - `calc:*` items must be registered in `ComputeRegistry`
    /// - `system:*` items are not yet supported
    ///
    /// `interval` 指定该 item 的计算频率。
    /// `reference_source` 为动态计算项提供参考圈数据来源（raw item 可传 `None`）。
    pub fn subscribe(
        &mut self,
        key: ItemKey,
        interval: Duration,
        reference_source: Option<ReferenceSource>,
    ) -> crate::compute::ComputeResult<()> {
        // Validate per type
        match key.item_type {
            ItemType::Raw => {
                if !TelemetryFrame::is_raw_field(&key.name) {
                    return Err(ComputeError::ItemNotFound(key.to_string()));
                }
            }
            ItemType::Calculated => {
                if !self.registry.is_registered(&key.name) {
                    return Err(ComputeError::ItemNotFound(key.to_string()));
                }
            }
            ItemType::System => {
                return Err(ComputeError::ItemNotFound(key.to_string()));
            }
        }

        self.subscriptions.insert(key.clone(), interval);
        self.next_schedule
            .insert(key.clone(), Instant::now() + interval);
        self.reference_sources
            .insert(key, reference_source);
        Ok(())
    }

    /// 取消订阅
    pub fn unsubscribe(&mut self, key: &ItemKey) {
        self.subscriptions.remove(key);
        self.next_schedule.remove(key);
        self.reference_sources.remove(key);
    }

    /// 获取当前订阅数
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// 原子替换全部订阅
    fn replace_all(&mut self, items: Vec<(ItemKey, Duration, Option<ReferenceSource>)>) {
        self.subscriptions.clear();
        self.next_schedule.clear();
        self.reference_sources.clear();
        for (key, interval, ref_src) in items {
            self.subscriptions.insert(key.clone(), interval);
            self.next_schedule.insert(key.clone(), Instant::now() + interval);
            self.reference_sources.insert(key, ref_src);
        }
    }

    /// 检查 item 是否已订阅
    pub fn is_subscribed(&self, key: &ItemKey) -> bool {
        self.subscriptions.contains_key(key)
    }

    /// 主运行循环
    ///
    /// 同时监听遥测帧 channel 和命令 channel。
    /// - 收到帧 → 按频率调度计算
    /// - 收到命令 → 动态增删订阅
    ///
    /// 两个 channel 的发送端都 drop 时，循环退出。
    pub fn run(
        &mut self,
        frame_rx: Receiver<Arc<TelemetryFrame>>,
        cmd_rx: Receiver<DashboardCommand>,
    ) {
        loop {
            let mut sel = Select::new();
            let frame_oper = sel.recv(&frame_rx);
            let cmd_oper = sel.recv(&cmd_rx);

            let oper = sel.select();
            match oper.index() {
                i if i == frame_oper => {
                    if let Ok(frame_arc) = oper.recv(&frame_rx) {
                        self.process_frame(&frame_arc);
                    } else {
                        break; // frame channel closed
                    }
                }
                i if i == cmd_oper => {
                    match oper.recv(&cmd_rx) {
                        Ok(DashboardCommand::Subscribe { item_key, interval, reference_source }) => {
                            if let Err(e) = self.subscribe(item_key.clone(), interval, reference_source) {
                                eprintln!("dashboard: cmd subscribe '{}' failed: {e}", item_key);
                            }
                        }
                        Ok(DashboardCommand::Unsubscribe(key)) => {
                            self.unsubscribe(&key);
                        }
                        Ok(DashboardCommand::ReplaceAll { items }) => {
                            self.replace_all(items);
                        }
                        Err(_) => break, // cmd channel closed
                    }
                }
                _ => break,
            }
        }
    }

    fn process_frame(&mut self, frame_arc: &Arc<TelemetryFrame>) {
        let now = Instant::now();
        let frame = &**frame_arc;

        if self.subscriptions.is_empty() {
            return;
        }

        // 收集本轮需要计算的 items
        let mut items_to_compute: Vec<ItemKey> = Vec::new();
        for (key, next_time) in &self.next_schedule {
            if now >= *next_time {
                items_to_compute.push(key.clone());
            }
        }

        if items_to_compute.is_empty() {
            return;
        }

        // 逐项计算
        let mut sparse_result = HashMap::new();
        for key in &items_to_compute {
            let value = self.compute_item(key, frame, &sparse_result);
            match value {
                Ok(val) => {
                    sparse_result.insert(key.to_string(), val);
                    if let Some(interval) = self.subscriptions.get(key) {
                        let prev = self.next_schedule.get(key).copied().unwrap_or(now);
                        self.next_schedule.insert(key.clone(), prev + *interval);
                    }
                }
                Err(err) => {
                    eprintln!("dashboard: compute item '{}' failed: {err}", key);
                }
            }
        }

        if !sparse_result.is_empty() {
            if let Err(err) = self.sink.send(sparse_result) {
                if !self.sink_error_reported {
                    eprintln!("dashboard sink send failed: {err}; subsequent errors suppressed");
                    self.sink_error_reported = true;
                }
            }
        }
    }

    /// 按类型分发计算
    fn compute_item(
        &mut self,
        key: &ItemKey,
        frame: &TelemetryFrame,
        computed_values: &HashMap<String, f64>,
    ) -> crate::compute::ComputeResult<f64> {
        match key.item_type {
            ItemType::Raw => {
                // 直接从帧读字段
                frame.raw_field_value(&key.name)
                    .ok_or_else(|| ComputeError::ItemNotFound(key.to_string()))
            }
            ItemType::Calculated => {
                // 解析参考圈
                let reference_arc = if let Some(Some(ref source)) = self.reference_sources.get(key) {
                    match self.registry.resolve_reference_lap(source) {
                        Ok(arc) => Some(arc),
                        Err(err) => {
                            eprintln!("dashboard: failed to load reference lap for '{key}': {err}");
                            None
                        }
                    }
                } else {
                    None
                };
                let reference_lap = reference_arc.as_ref().map(|arc| arc.as_slice());

                let request = RealtimeComputeRequest {
                    current_frame: frame,
                    computed_values,
                    reference_lap,
                    reference_source: self
                        .reference_sources
                        .get(key)
                        .and_then(|r| r.clone()),
                };
                self.registry.compute_realtime(&key.name, &request)
            }
            ItemType::System => {
                Err(ComputeError::ItemNotFound(key.to_string()))
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

    /// 列出所有可用的 dashboard item（raw + calculated）
    pub fn list_available_items(&self) -> Vec<crate::recording::DashboardItemInfo> {
        use crate::recording::{DashboardItemInfo, DashboardItemKind};
        let mut items = Vec::new();

        // Raw items — 从中文描述目录获取
        for entry in crate::raw_catalog::all_raw_items() {
            items.push(DashboardItemInfo {
                name: entry.key.to_string(),
                kind: DashboardItemKind::RawItem,
                description: entry.description.clone(),
                unit: entry.unit.map(|u| u.to_string()),
            });
        }

        // Calculated items from registry
        for name in self.registry.registered_item_names() {
            items.push(DashboardItemInfo {
                name: format!("calc:{}", name),
                kind: DashboardItemKind::CalculatedItem,
                description: format!("Calculated item: {}", name),
                unit: None,
            });
        }

        items
    }

    /// 检查 item 是否可用（raw 或 calculated）
    pub fn is_item_available(&self, item_name: &str) -> bool {
        if let Some(key) = ItemKey::parse(item_name) {
            match key.item_type {
                ItemType::Raw => TelemetryFrame::is_raw_field(&key.name),
                ItemType::Calculated => self.registry.is_registered(&key.name),
                ItemType::System => false,
            }
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::items::RealtimeComputeItem;
    use crate::compute::{ComputeContext, ComputeResult};
    use crate::dashboard::sink::ChannelSink;

    /// 本地测试用计算项
    struct TestCalcItem;
    impl RealtimeComputeItem for TestCalcItem {
        fn name(&self) -> &str { "test_calc" }
        fn compute(&mut self, _ctx: &ComputeContext) -> ComputeResult<f64> {
            Ok(42.0)
        }
    }
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
                gas: 0.5,
                brake: 0.1,
                clutch: 0.0,
                steer_angle: 0.0,
                gear: 3,
                rpms: 6000,
                fuel: 50.0,
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
    fn test_subscribe_raw_item() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<HashMap<String, f64>>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
        assert!(service.subscribe(key, Duration::from_millis(50), None).is_ok());
    }

    #[test]
    fn test_subscribe_raw_item_invalid() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<HashMap<String, f64>>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("raw:controls.nonexistent").unwrap();
        assert!(service.subscribe(key, Duration::from_millis(50), None).is_err());
    }

    #[test]
    fn test_subscribe_calc_item() {
        let mut registry = ComputeRegistry::new();
        registry.register_calc_realtime(Box::new(TestCalcItem)).unwrap();
        let (tx, _rx) = bounded::<HashMap<String, f64>>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("calc:test_calc").unwrap();
        assert!(service.subscribe(key, Duration::from_millis(50), None).is_ok());
    }

    #[test]
    fn test_subscribe_calc_item_not_registered() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<HashMap<String, f64>>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("calc:nonexistent").unwrap();
        assert!(service.subscribe(key, Duration::from_millis(50), None).is_err());
    }

    #[test]
    fn test_is_item_available() {
        let mut registry = ComputeRegistry::new();
        registry.register_calc_realtime(Box::new(TestCalcItem)).unwrap();
        let (tx, _rx) = bounded::<HashMap<String, f64>>(1);
        let sink = ChannelSink::new(tx);
        let service = DashboardService::new(registry, Box::new(sink));

        assert!(service.is_item_available("raw:controls.speed_kmh"));
        assert!(service.is_item_available("calc:test_calc"));
        assert!(!service.is_item_available("calc:nonexistent"));
        assert!(!service.is_item_available("raw:controls.nonexistent"));
    }

    #[test]
    fn test_compute_raw_item_from_frame() {
        let registry = ComputeRegistry::new();
        let (tx, rx) = bounded::<HashMap<String, f64>>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
        service.subscribe(key.clone(), Duration::from_millis(0), None).unwrap();

        let frame = make_frame(100.0);
        let (send_tx, send_rx) = bounded::<Arc<TelemetryFrame>>(1);
        send_tx.send(Arc::new(frame)).unwrap();
        drop(send_tx);

        let (_cmd_tx, cmd_rx) = bounded::<DashboardCommand>(1);
        service.run(send_rx, cmd_rx);

        let result = rx.recv().unwrap();
        let expected_key = key.to_string();
        assert!((result[&expected_key] - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_unsubscribe() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<HashMap<String, f64>>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
        service.subscribe(key.clone(), Duration::from_millis(50), None).unwrap();
        assert!(service.is_subscribed(&key));
        service.unsubscribe(&key);
        assert!(!service.is_subscribed(&key));
    }
}
