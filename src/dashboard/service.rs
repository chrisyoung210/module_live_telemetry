//! Dashboard 数据服务
//!
//! 管理计算项订阅、按频率调度计算、聚合结果并通过 DataSink 回传。
//!
//! 支持三种 item 类型：
//! - `raw:*` — 从 TelemetryFrame 字段直接读取，无需注册
//! - `calc:*` — 通过 ComputeRegistry 计算
//! - `system:*` — 系统信息（未来）

use crate::compute::context::ReferenceSource;
use crate::compute::{ComputeError, ComputeRegistry, RealtimeComputeRequest};
use crate::dashboard::sink::DataSink;
use crate::item_key::{ItemKey, ItemType};
use crate::recording::dashboard::{
    dashboard_item_info_for_subscription, validate_dashboard_subscriptions_with_registry,
};
use crate::recording::{
    DashboardItemInfo, DashboardItemKind, DashboardItemSubscription,
    DashboardSubscriptionGeneration, DashboardSubscriptionValidation, DashboardValuesFrame,
};
use crate::TelemetryFrame;
use crossbeam_channel::{Receiver, Select, Sender};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        ack: Sender<
            Result<DashboardSubscriptionGeneration, crate::recording::DashboardSubscriptionError>,
        >,
    },
    /// Replace a runtime reference lap used by calculated dashboard items.
    ReplaceReference {
        source: ReferenceSource,
        lap_number: u32,
        lap_time_ms: i32,
        frames: Vec<TelemetryFrame>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DashboardServiceStats {
    pub input_frames: u64,
    pub produced_frames: u64,
    pub sink_dropped_frames: u64,
    pub computed_values: u64,
    pub compute_errors: u64,
    pub subscription_replacements: u64,
    pub last_sample_tick: u64,
    pub last_timestamp_ns: u64,
    pub last_compute_duration_ns: u64,
    pub max_compute_duration_ns: u64,
    pub subscription_generation: DashboardSubscriptionGeneration,
}

#[derive(Debug, Clone, Default)]
pub struct DashboardServiceStatsHandle {
    inner: Arc<DashboardServiceStatsInner>,
}

#[derive(Debug, Default)]
struct DashboardServiceStatsInner {
    input_frames: AtomicU64,
    produced_frames: AtomicU64,
    sink_dropped_frames: AtomicU64,
    computed_values: AtomicU64,
    compute_errors: AtomicU64,
    subscription_replacements: AtomicU64,
    last_sample_tick: AtomicU64,
    last_timestamp_ns: AtomicU64,
    last_compute_duration_ns: AtomicU64,
    max_compute_duration_ns: AtomicU64,
    subscription_generation: AtomicU64,
}

impl DashboardServiceStatsHandle {
    pub fn snapshot(&self) -> DashboardServiceStats {
        DashboardServiceStats {
            input_frames: self.inner.input_frames.load(Ordering::Relaxed),
            produced_frames: self.inner.produced_frames.load(Ordering::Relaxed),
            sink_dropped_frames: self.inner.sink_dropped_frames.load(Ordering::Relaxed),
            computed_values: self.inner.computed_values.load(Ordering::Relaxed),
            compute_errors: self.inner.compute_errors.load(Ordering::Relaxed),
            subscription_replacements: self.inner.subscription_replacements.load(Ordering::Relaxed),
            last_sample_tick: self.inner.last_sample_tick.load(Ordering::Relaxed),
            last_timestamp_ns: self.inner.last_timestamp_ns.load(Ordering::Relaxed),
            last_compute_duration_ns: self.inner.last_compute_duration_ns.load(Ordering::Relaxed),
            max_compute_duration_ns: self.inner.max_compute_duration_ns.load(Ordering::Relaxed),
            subscription_generation: self.inner.subscription_generation.load(Ordering::Acquire),
        }
    }
}

struct ScheduleBucket {
    next_due: Instant,
    items: Vec<ItemKey>,
}

#[derive(Debug, Clone)]
enum RawFieldSource {
    TopLevel,
    Controls,
    Motion,
    Tyres,
    Powertrain,
    Session,
    Timing,
    CarState,
    Environment,
    OtherCars,
}

#[derive(Debug, Clone)]
struct RawFieldAccessor {
    source: RawFieldSource,
    field: String,
}

impl RawFieldAccessor {
    fn compile(path: &str) -> Option<Self> {
        let (source, field) = match path.split_once('.') {
            Some(("controls", field)) => (RawFieldSource::Controls, field),
            Some(("motion", field)) => (RawFieldSource::Motion, field),
            Some(("tyres", field)) => (RawFieldSource::Tyres, field),
            Some(("powertrain", field)) => (RawFieldSource::Powertrain, field),
            Some(("session", field)) => (RawFieldSource::Session, field),
            Some(("timing", field)) => (RawFieldSource::Timing, field),
            Some(("car_state", field)) => (RawFieldSource::CarState, field),
            Some(("environment", field)) => (RawFieldSource::Environment, field),
            Some(("other_cars", field)) => (RawFieldSource::OtherCars, field),
            None if matches!(path, "sample_tick" | "timestamp_ns") => {
                (RawFieldSource::TopLevel, path)
            }
            _ => return None,
        };
        Some(Self {
            source,
            field: field.to_string(),
        })
    }

    fn read(&self, frame: &TelemetryFrame) -> Option<f64> {
        match self.source {
            RawFieldSource::TopLevel => match self.field.as_str() {
                "sample_tick" => Some(frame.sample_tick as f64),
                "timestamp_ns" => Some(frame.timestamp_ns as f64),
                _ => None,
            },
            RawFieldSource::Controls => frame.controls.raw_field_value(&self.field),
            RawFieldSource::Motion => frame.motion.raw_field_value(&self.field),
            RawFieldSource::Tyres => frame.tyres.raw_field_value(&self.field),
            RawFieldSource::Powertrain => frame.powertrain.raw_field_value(&self.field),
            RawFieldSource::Session => frame.session.raw_field_value(&self.field),
            RawFieldSource::Timing => frame.timing.raw_field_value(&self.field),
            RawFieldSource::CarState => frame.car_state.raw_field_value(&self.field),
            RawFieldSource::Environment => frame.environment.raw_field_value(&self.field),
            RawFieldSource::OtherCars => frame.other_cars.raw_field_value(&self.field),
        }
    }
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
    /// Items grouped by interval, reducing per-frame scheduling work.
    schedule_buckets: HashMap<Duration, ScheduleBucket>,
    /// Pre-parsed raw field accessors, avoiding path parsing on every frame.
    raw_accessors: HashMap<ItemKey, RawFieldAccessor>,
    /// item_key → 参考圈数据来源（None 表示不需要参考圈）
    reference_sources: HashMap<ItemKey, Option<ReferenceSource>>,
    /// 是否已经报告过 sink 发送错误（每个 service 实例只报告一次）
    sink_error_reported: bool,
    /// Last time each item compute failure was logged, to avoid flooding stderr.
    last_compute_error_log: HashMap<ItemKey, Instant>,
    /// Whether each item has emitted at least one successful value.
    item_has_output: HashMap<ItemKey, bool>,
    /// Total dashboard frames successfully sent to the sink.
    sent_frame_count: u64,
    /// Temporary sampled CSV trace for investigating dashboard values.
    debug_trace: DashboardDebugTrace,
    stats: DashboardServiceStatsHandle,
    subscription_generation: DashboardSubscriptionGeneration,
}

impl DashboardService {
    /// 创建新的 Dashboard 服务
    pub fn new(registry: ComputeRegistry, sink: Box<dyn DataSink>) -> Self {
        Self::with_stats(registry, sink, DashboardServiceStatsHandle::default())
    }

    pub fn with_stats(
        registry: ComputeRegistry,
        sink: Box<dyn DataSink>,
        stats: DashboardServiceStatsHandle,
    ) -> Self {
        Self {
            registry,
            sink,
            subscriptions: HashMap::new(),
            schedule_buckets: HashMap::new(),
            raw_accessors: HashMap::new(),
            reference_sources: HashMap::new(),
            sink_error_reported: false,
            last_compute_error_log: HashMap::new(),
            item_has_output: HashMap::new(),
            sent_frame_count: 0,
            debug_trace: DashboardDebugTrace::from_env(),
            stats,
            subscription_generation: 0,
        }
    }

    pub fn stats_handle(&self) -> DashboardServiceStatsHandle {
        self.stats.clone()
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

        let reference_source = reference_source_for_subscription(&key, reference_source);
        self.subscriptions.insert(key.clone(), interval);
        if key.item_type == ItemType::Raw {
            if let Some(accessor) = RawFieldAccessor::compile(&key.name) {
                self.raw_accessors.insert(key.clone(), accessor);
            }
        }
        let has_reference_source = reference_source.is_some();
        self.reference_sources.insert(key.clone(), reference_source);
        self.item_has_output.insert(key.clone(), false);
        self.rebuild_schedule_buckets();
        self.advance_subscription_generation();
        eprintln!(
            "dashboard: subscribed '{}' interval={}ms reference_source={}",
            key,
            interval.as_millis(),
            has_reference_source
        );
        Ok(())
    }

    /// 取消订阅
    pub fn unsubscribe(&mut self, key: &ItemKey) {
        if self.subscriptions.remove(key).is_none() {
            return;
        }
        self.raw_accessors.remove(key);
        self.reference_sources.remove(key);
        self.last_compute_error_log.remove(key);
        self.item_has_output.remove(key);
        self.rebuild_schedule_buckets();
        self.advance_subscription_generation();
        eprintln!("dashboard: unsubscribed '{}'", key);
    }

    /// 获取当前订阅数
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// 原子替换全部订阅
    fn replace_all(
        &mut self,
        items: Vec<(ItemKey, Duration, Option<ReferenceSource>)>,
    ) -> Result<DashboardSubscriptionGeneration, crate::recording::DashboardSubscriptionError> {
        let subscriptions: Vec<_> = items
            .iter()
            .map(
                |(key, interval, reference_source)| DashboardItemSubscription {
                    item_name: key.to_string(),
                    item_kind: dashboard_kind_for_key(key),
                    interval: *interval,
                    reference_source: reference_source.clone(),
                },
            )
            .collect();
        let validation =
            validate_dashboard_subscriptions_with_registry(&subscriptions, &self.registry);
        if let Some(error) = validation.errors.into_iter().next() {
            eprintln!(
                "dashboard: replace subscriptions rejected for '{}': {}",
                error.item_name, error.message
            );
            return Err(error);
        }

        self.subscriptions.clear();
        self.schedule_buckets.clear();
        self.raw_accessors.clear();
        self.reference_sources.clear();
        self.last_compute_error_log.clear();
        self.item_has_output.clear();
        for (key, interval, ref_src) in items {
            let ref_src = reference_source_for_subscription(&key, ref_src);
            let has_reference_source = ref_src.is_some();
            self.subscriptions.insert(key.clone(), interval);
            if key.item_type == ItemType::Raw {
                if let Some(accessor) = RawFieldAccessor::compile(&key.name) {
                    self.raw_accessors.insert(key.clone(), accessor);
                }
            }
            self.reference_sources.insert(key.clone(), ref_src);
            self.item_has_output.insert(key.clone(), false);
            eprintln!(
                "dashboard: replace subscribed '{}' interval={}ms reference_source={}",
                key,
                interval.as_millis(),
                has_reference_source
            );
        }
        self.rebuild_schedule_buckets();
        self.stats
            .inner
            .subscription_replacements
            .fetch_add(1, Ordering::Relaxed);
        self.advance_subscription_generation();
        eprintln!(
            "dashboard: replaced subscriptions total={}",
            self.subscriptions.len()
        );
        Ok(self.subscription_generation)
    }

    /// 检查 item 是否已订阅
    pub fn is_subscribed(&self, key: &ItemKey) -> bool {
        self.subscriptions.contains_key(key)
    }

    /// List dashboard items currently subscribed by this service.
    pub fn list_dashboard_items(&self) -> Vec<DashboardItemInfo> {
        self.subscriptions
            .iter()
            .map(|(key, interval)| {
                let subscription = DashboardItemSubscription {
                    item_name: key.to_string(),
                    item_kind: dashboard_kind_for_key(key),
                    interval: *interval,
                    reference_source: self.reference_sources.get(key).and_then(|s| s.clone()),
                };
                dashboard_item_info_for_subscription(&subscription)
            })
            .collect()
    }

    /// Validate subscriptions against this service's registry without mutating state.
    pub fn validate_dashboard_subscriptions(
        &self,
        items: &[DashboardItemSubscription],
    ) -> DashboardSubscriptionValidation {
        validate_dashboard_subscriptions_with_registry(items, &self.registry)
    }

    fn rebuild_schedule_buckets(&mut self) {
        let now = Instant::now();
        let mut buckets: HashMap<Duration, ScheduleBucket> = HashMap::new();
        for (key, interval) in &self.subscriptions {
            buckets
                .entry(*interval)
                .or_insert_with(|| ScheduleBucket {
                    next_due: now + *interval,
                    items: Vec::new(),
                })
                .items
                .push(key.clone());
        }
        self.schedule_buckets = buckets;
    }

    fn advance_subscription_generation(&mut self) {
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        self.stats
            .inner
            .subscription_generation
            .store(self.subscription_generation, Ordering::Release);
        self.sink
            .advance_subscription_generation(self.subscription_generation);
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
        eprintln!(
            "dashboard: run loop started subscriptions={}",
            self.subscriptions.len()
        );
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
                        eprintln!("dashboard: frame channel closed; stopping run loop");
                        break; // frame channel closed
                    }
                }
                i if i == cmd_oper => {
                    match oper.recv(&cmd_rx) {
                        Ok(DashboardCommand::Subscribe {
                            item_key,
                            interval,
                            reference_source,
                        }) => {
                            if let Err(e) =
                                self.subscribe(item_key.clone(), interval, reference_source)
                            {
                                eprintln!("dashboard: cmd subscribe '{}' failed: {e}", item_key);
                            }
                        }
                        Ok(DashboardCommand::Unsubscribe(key)) => {
                            self.unsubscribe(&key);
                        }
                        Ok(DashboardCommand::ReplaceAll { items, ack }) => {
                            let result = self.replace_all(items);
                            if let Err(err) = &result {
                                eprintln!(
                                    "dashboard: cmd replace subscriptions failed for '{}': {}",
                                    err.item_name, err.message
                                );
                            }
                            let _ = ack.send(result);
                        }
                        Ok(DashboardCommand::ReplaceReference {
                            source,
                            lap_number,
                            lap_time_ms,
                            frames,
                        }) => {
                            self.replace_reference(source, lap_number, lap_time_ms, frames);
                        }
                        Err(_) => {
                            eprintln!("dashboard: command channel closed; stopping run loop");
                            break; // cmd channel closed
                        }
                    }
                }
                _ => {
                    eprintln!("dashboard: select returned unexpected operation; stopping run loop");
                    break;
                }
            }
        }
        eprintln!("dashboard: run loop stopped");
    }

    fn process_frame(&mut self, frame_arc: &Arc<TelemetryFrame>) {
        let now = Instant::now();
        let compute_started = now;
        let frame = &**frame_arc;
        self.stats
            .inner
            .input_frames
            .fetch_add(1, Ordering::Relaxed);
        self.stats
            .inner
            .last_sample_tick
            .store(frame.sample_tick, Ordering::Relaxed);
        self.stats
            .inner
            .last_timestamp_ns
            .store(frame.timestamp_ns, Ordering::Relaxed);

        if self.subscriptions.is_empty() {
            return;
        }

        // 收集本轮需要计算的 items
        let mut items_to_compute: Vec<ItemKey> = Vec::new();
        for (interval, bucket) in &mut self.schedule_buckets {
            if now >= bucket.next_due {
                items_to_compute.extend(bucket.items.iter().cloned());
                if interval.is_zero() {
                    bucket.next_due = now;
                } else {
                    while bucket.next_due <= now {
                        bucket.next_due += *interval;
                    }
                }
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
                    self.stats
                        .inner
                        .computed_values
                        .fetch_add(1, Ordering::Relaxed);
                    if !self.item_has_output.get(key).copied().unwrap_or(false) {
                        eprintln!(
                            "dashboard: first value for '{}' sample_tick={} value={:.4}",
                            key, frame.sample_tick, val
                        );
                        self.item_has_output.insert(key.clone(), true);
                    }
                    self.last_compute_error_log.remove(key);
                }
                Err(err) => {
                    self.stats
                        .inner
                        .compute_errors
                        .fetch_add(1, Ordering::Relaxed);
                    if self.should_log_compute_error(key, now) {
                        let has_reference_source = self
                            .reference_sources
                            .get(key)
                            .and_then(|source| source.as_ref())
                            .is_some();
                        eprintln!(
                            "dashboard: compute item '{}' failed at sample_tick={} reference_source={}: {err}",
                            key, frame.sample_tick, has_reference_source
                        );
                    }
                }
            }
        }

        if !sparse_result.is_empty() {
            let value_count = sparse_result.len();
            let dashboard_frame = DashboardValuesFrame {
                subscription_generation: self.subscription_generation,
                sample_tick: frame.sample_tick,
                timestamp_ns: frame.timestamp_ns,
                values: sparse_result,
            };
            if let Err(err) = self.sink.send(dashboard_frame.clone()) {
                self.stats
                    .inner
                    .sink_dropped_frames
                    .fetch_add(1, Ordering::Relaxed);
                if !self.sink_error_reported {
                    eprintln!("dashboard sink send failed: {err}; subsequent errors suppressed");
                    self.sink_error_reported = true;
                }
            } else {
                self.sent_frame_count = self.sent_frame_count.saturating_add(1);
                self.stats
                    .inner
                    .produced_frames
                    .fetch_add(1, Ordering::Relaxed);
                self.debug_trace
                    .write_frame(self.sent_frame_count, &dashboard_frame, value_count);
                if self.sent_frame_count == 1 || self.sent_frame_count.is_multiple_of(1000) {
                    eprintln!(
                        "dashboard: sent frame #{} sample_tick={} values={}",
                        self.sent_frame_count, frame.sample_tick, value_count
                    );
                }
            }
        }
        let elapsed_ns = compute_started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        self.stats
            .inner
            .last_compute_duration_ns
            .store(elapsed_ns, Ordering::Relaxed);
        self.stats
            .inner
            .max_compute_duration_ns
            .fetch_max(elapsed_ns, Ordering::Relaxed);
    }

    fn should_log_compute_error(&mut self, key: &ItemKey, now: Instant) -> bool {
        const COMPUTE_ERROR_LOG_INTERVAL: Duration = Duration::from_secs(5);
        match self.last_compute_error_log.get(key).copied() {
            Some(last) if now.duration_since(last) < COMPUTE_ERROR_LOG_INTERVAL => false,
            _ => {
                self.last_compute_error_log.insert(key.clone(), now);
                true
            }
        }
    }

    fn replace_reference(
        &mut self,
        source: ReferenceSource,
        lap_number: u32,
        lap_time_ms: i32,
        frames: Vec<TelemetryFrame>,
    ) {
        let frame_count = frames.len();
        self.registry.replace_reference(source.clone(), frames);
        eprintln!(
            "dashboard: replaced reference source='{}' lap={} lap_time_ms={} frames={}",
            reference_source_label(&source),
            lap_number,
            lap_time_ms,
            frame_count
        );
    }

    /// 按类型分发计算
    fn compute_item(
        &mut self,
        key: &ItemKey,
        frame: &TelemetryFrame,
        computed_values: &HashMap<String, f64>,
    ) -> crate::compute::ComputeResult<f64> {
        match key.item_type {
            ItemType::Raw => self
                .raw_accessors
                .get(key)
                .and_then(|accessor| accessor.read(frame))
                .ok_or_else(|| ComputeError::ItemNotFound(key.to_string())),
            ItemType::Calculated => {
                // 解析参考圈
                let reference_arc = if let Some(Some(ref source)) = self.reference_sources.get(key)
                {
                    match self.registry.resolve_reference_lap(source) {
                        Ok(arc) => Some(arc),
                        Err(ComputeError::NoValidData) if source.is_session_best() => {
                            None
                        }
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
                    reference_source: self.reference_sources.get(key).and_then(|r| r.clone()),
                };
                self.registry.compute_realtime(&key.name, &request)
            }
            ItemType::System => Err(ComputeError::ItemNotFound(key.to_string())),
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

struct DashboardDebugTrace {
    writer: Option<BufWriter<File>>,
    gap: u64,
    error_reported: bool,
}

impl DashboardDebugTrace {
    fn from_env() -> Self {
        if trace_disabled() {
            return Self {
                writer: None,
                gap: 0,
                error_reported: false,
            };
        }

        let gap = std::env::var("ACC_DASHBOARD_TRACE_GAP")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(60);
        let path = std::env::var_os("ACC_DASHBOARD_TRACE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("acc-dashboard-values.csv"));

        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => {
                let needs_header = file.metadata().map(|meta| meta.len() == 0).unwrap_or(false);
                let mut writer = BufWriter::new(file);
                if needs_header {
                    let _ = writeln!(
                        writer,
                        "wall_time_ms,sent_frame_count,sample_tick,timestamp_ns,value_count,item,value"
                    );
                }
                eprintln!(
                    "dashboard trace: writing sampled values path='{}' gap={}",
                    path.display(),
                    gap
                );
                Self {
                    writer: Some(writer),
                    gap,
                    error_reported: false,
                }
            }
            Err(err) => {
                eprintln!(
                    "dashboard trace: failed to open path='{}': {err}",
                    path.display()
                );
                Self {
                    writer: None,
                    gap,
                    error_reported: true,
                }
            }
        }
    }

    fn write_frame(
        &mut self,
        sent_frame_count: u64,
        frame: &DashboardValuesFrame,
        value_count: usize,
    ) {
        if self.writer.is_none() || self.gap == 0 {
            return;
        }
        if sent_frame_count != 1 && !sent_frame_count.is_multiple_of(self.gap) {
            return;
        }

        let result = write_trace_rows(
            self.writer.as_mut().expect("writer checked above"),
            sent_frame_count,
            frame,
            value_count,
        );
        if let Err(err) = result {
            self.report_write_error(err);
        }
    }

    fn report_write_error(&mut self, err: std::io::Error) {
        if !self.error_reported {
            eprintln!("dashboard trace: write failed: {err}; disabling trace");
            self.error_reported = true;
        }
        self.writer = None;
    }
}

fn write_trace_rows(
    writer: &mut BufWriter<File>,
    sent_frame_count: u64,
    frame: &DashboardValuesFrame,
    value_count: usize,
) -> std::io::Result<()> {
    let wall_time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    if frame.values.is_empty() {
        writeln!(
            writer,
            "{wall_time_ms},{sent_frame_count},{},{},{value_count},,",
            frame.sample_tick, frame.timestamp_ns
        )?;
        return writer.flush();
    }

    let mut values: Vec<_> = frame.values.iter().collect();
    values.sort_by_key(|(name, _)| *name);
    for (item, value) in values {
        writeln!(
            writer,
            "{wall_time_ms},{sent_frame_count},{},{},{value_count},{},{}",
            frame.sample_tick,
            frame.timestamp_ns,
            csv_cell(item),
            value
        )?;
    }
    writer.flush()
}

fn trace_disabled() -> bool {
    std::env::var("ACC_DASHBOARD_TRACE")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            matches!(value.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(false)
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn dashboard_kind_for_key(key: &ItemKey) -> DashboardItemKind {
    match key.item_type {
        ItemType::Raw => DashboardItemKind::RawItem,
        ItemType::Calculated => DashboardItemKind::CalculatedItem,
        ItemType::System => DashboardItemKind::SystemItem,
    }
}

fn reference_source_for_subscription(
    key: &ItemKey,
    provided: Option<ReferenceSource>,
) -> Option<ReferenceSource> {
    if key.item_type == ItemType::Calculated
        && matches!(
            key.name.as_str(),
            "delta_time_to_session_best_lap" | "delta_time_to_session_best_lap_interpolated"
        )
    {
        Some(ReferenceSource::session_best())
    } else {
        provided
    }
}

fn reference_source_label(source: &ReferenceSource) -> String {
    if source.is_session_best() {
        "session_best".to_string()
    } else {
        format!("{}#{}", source.file_path.display(), source.lap_number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::items::RealtimeComputeItem;
    use crate::compute::{ComputeContext, ComputeResult};
    use crate::dashboard::sink::{latest_value_channel, ChannelSink, LatestValueSink};

    /// 本地测试用计算项
    struct TestCalcItem;
    impl RealtimeComputeItem for TestCalcItem {
        fn name(&self) -> &str {
            "test_calc"
        }
        fn compute(&mut self, _ctx: &ComputeContext) -> ComputeResult<f64> {
            Ok(42.0)
        }
    }
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
        PowertrainSample, SessionSample, TimingSample, TyreSample,
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
        let (tx, _rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
        assert!(service
            .subscribe(key, Duration::from_millis(50), None)
            .is_ok());
    }

    #[test]
    fn test_subscribe_raw_item_invalid() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("raw:controls.nonexistent").unwrap();
        assert!(service
            .subscribe(key, Duration::from_millis(50), None)
            .is_err());
    }

    #[test]
    fn test_subscribe_calc_item() {
        let mut registry = ComputeRegistry::new();
        registry
            .register_calc_realtime(Box::new(TestCalcItem))
            .unwrap();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("calc:test_calc").unwrap();
        assert!(service
            .subscribe(key, Duration::from_millis(50), None)
            .is_ok());
    }

    #[test]
    fn test_session_best_subscription_uses_internal_reference() {
        let registry = ComputeRegistry::with_builtin_dashboard_items().unwrap();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("calc:delta_time_to_session_best_lap").unwrap();
        let external_reference = ReferenceSource {
            file_path: std::path::PathBuf::from("external.acctlm2"),
            lap_number: 7,
        };

        service
            .subscribe(
                key.clone(),
                Duration::from_millis(50),
                Some(external_reference),
            )
            .unwrap();

        let source = service
            .reference_sources
            .get(&key)
            .and_then(|source| source.as_ref())
            .unwrap();
        assert!(source.is_session_best());
    }

    #[test]
    fn test_interpolated_session_best_subscription_uses_internal_reference() {
        let registry = ComputeRegistry::with_builtin_dashboard_items().unwrap();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("calc:delta_time_to_session_best_lap_interpolated").unwrap();
        service
            .subscribe(key.clone(), Duration::from_millis(50), None)
            .unwrap();

        let source = service
            .reference_sources
            .get(&key)
            .and_then(|source| source.as_ref())
            .unwrap();
        assert!(source.is_session_best());
    }

    #[test]
    fn test_subscribe_calc_item_not_registered() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("calc:nonexistent").unwrap();
        assert!(service
            .subscribe(key, Duration::from_millis(50), None)
            .is_err());
    }

    #[test]
    fn test_is_item_available() {
        let mut registry = ComputeRegistry::new();
        registry
            .register_calc_realtime(Box::new(TestCalcItem))
            .unwrap();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(1);
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
        let (tx, rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
        service
            .subscribe(key.clone(), Duration::from_millis(0), None)
            .unwrap();

        let frame = make_frame(100.0);
        let (send_tx, send_rx) = bounded::<Arc<TelemetryFrame>>(1);
        send_tx.send(Arc::new(frame)).unwrap();
        drop(send_tx);

        let (_cmd_tx, cmd_rx) = bounded::<DashboardCommand>(1);
        service.run(send_rx, cmd_rx);

        let result = rx.recv().unwrap();
        let expected_key = key.to_string();
        assert_eq!(result.sample_tick, 0);
        assert_eq!(result.timestamp_ns, 0);
        assert!((result.values[&expected_key] - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_unsubscribe() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);
        let mut service = DashboardService::new(registry, Box::new(sink));

        let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
        service
            .subscribe(key.clone(), Duration::from_millis(50), None)
            .unwrap();
        assert!(service.is_subscribed(&key));
        service.unsubscribe(&key);
        assert!(!service.is_subscribed(&key));
    }

    #[test]
    fn subscriptions_with_same_interval_share_a_schedule_bucket() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(4);
        let mut service = DashboardService::new(registry, Box::new(ChannelSink::new(tx)));
        for (name, interval) in [
            ("raw:controls.speed_kmh", 16),
            ("raw:controls.rpms", 16),
            ("raw:controls.gear", 50),
        ] {
            service
                .subscribe(
                    ItemKey::parse(name).unwrap(),
                    Duration::from_millis(interval),
                    None,
                )
                .unwrap();
        }

        assert_eq!(service.schedule_buckets.len(), 2);
        assert_eq!(
            service.schedule_buckets[&Duration::from_millis(16)]
                .items
                .len(),
            2
        );
        assert_eq!(service.raw_accessors.len(), 3);
    }

    #[test]
    fn stats_count_produced_values() {
        let registry = ComputeRegistry::new();
        let (tx, _rx) = bounded::<DashboardValuesFrame>(4);
        let stats = DashboardServiceStatsHandle::default();
        let mut service =
            DashboardService::with_stats(registry, Box::new(ChannelSink::new(tx)), stats.clone());
        service
            .subscribe(
                ItemKey::parse("raw:controls.speed_kmh").unwrap(),
                Duration::ZERO,
                None,
            )
            .unwrap();
        service.process_frame(&Arc::new(make_frame(88.0)));

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.input_frames, 1);
        assert_eq!(snapshot.produced_frames, 1);
        assert_eq!(snapshot.sink_dropped_frames, 0);
        assert_eq!(snapshot.computed_values, 1);
        assert_eq!(snapshot.compute_errors, 0);
        assert_eq!(snapshot.last_sample_tick, 0);
        assert_eq!(snapshot.last_timestamp_ns, 0);
        assert!(snapshot.max_compute_duration_ns >= snapshot.last_compute_duration_ns);
    }

    #[test]
    fn replace_ack_follows_apply_and_pending_clear() {
        let registry = ComputeRegistry::new();
        let (latest_tx, latest_rx) = latest_value_channel();
        let mut service =
            DashboardService::new(registry, Box::new(LatestValueSink::new(latest_tx)));
        service
            .subscribe(
                ItemKey::parse("raw:controls.speed_kmh").unwrap(),
                Duration::ZERO,
                None,
            )
            .unwrap();
        service.process_frame(&Arc::new(make_frame(88.0)));

        let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(2);
        let (cmd_tx, cmd_rx) = bounded::<DashboardCommand>(2);
        let handle = std::thread::spawn(move || service.run(frame_rx, cmd_rx));
        let (ack_tx, ack_rx) = bounded(1);
        cmd_tx
            .send(DashboardCommand::ReplaceAll {
                items: vec![(
                    ItemKey::parse("raw:controls.gear").unwrap(),
                    Duration::from_millis(1),
                    None,
                )],
                ack: ack_tx,
            })
            .unwrap();

        let generation = ack_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert!(matches!(
            latest_rx.try_recv(),
            Err(crossbeam_channel::TryRecvError::Empty)
        ));

        std::thread::sleep(Duration::from_millis(5));
        frame_tx.send(Arc::new(make_frame(99.0))).unwrap();
        let frame = latest_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(frame.subscription_generation, generation);
        assert_eq!(frame.values.len(), 1);
        assert_eq!(frame.values.get("raw:controls.gear"), Some(&3.0));

        drop(frame_tx);
        drop(cmd_tx);
        handle.join().unwrap();
    }
}
