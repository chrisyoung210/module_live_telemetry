//! Dashboard 数据服务集成测试
//!
//! 测试 DashboardService 的公共 API：订阅验证、数据流端到端。
//! 使用本地定义的测试 item 替代已移除的 SpeedMps。

use module_live_telemetry::{
    TelemetryFrame,
    compute::{ComputeContext, ComputeResult, ComputeRegistry, items::RealtimeComputeItem},
    dashboard::service::{DashboardService, DashboardCommand},
    dashboard::sink::ChannelSink,
    item_key::ItemKey,
    types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    },
};
use crossbeam_channel::bounded;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// 本地测试用计算项：km/h → m/s
struct TestSpeedItem;
impl RealtimeComputeItem for TestSpeedItem {
    fn name(&self) -> &str { "test_speed" }
    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        Ok(ctx.current_frame.controls.speed_kmh as f64 / 3.6)
    }
}

fn make_frame(speed: f32) -> TelemetryFrame {
    TelemetryFrame {
        sample_tick: 0, timestamp_ns: 0,
        controls: ControlSample { speed_kmh: speed, ..ControlSample::default() },
        motion: MotionSample::default(), tyres: TyreSample::default(),
        powertrain: PowertrainSample::default(), session: SessionSample::default(),
        timing: TimingSample::default(), car_state: CarStateSample::default(),
        environment: EnvironmentSample::default(), other_cars: OtherCarsSample::default(),
    }
}

#[test]
fn subscribe_unknown_item_returns_error() {
    let reg = ComputeRegistry::new();
    let (tx, _rx) = bounded::<HashMap<String, f64>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let key = ItemKey::parse("calc:nonexistent").unwrap();
    let result = service.subscribe(key, Duration::from_millis(100), None);
    assert!(result.is_err());
    assert_eq!(service.subscription_count(), 0);
}

#[test]
fn subscribe_registered_item_succeeds() {
    let mut reg = ComputeRegistry::new();
    reg.register_calc_realtime(Box::new(TestSpeedItem)).unwrap();
    let (tx, _rx) = bounded::<HashMap<String, f64>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let key = ItemKey::parse("calc:test_speed").unwrap();
    let result = service.subscribe(key.clone(), Duration::from_millis(100), None);
    assert!(result.is_ok());
    assert!(service.is_subscribed(&key));
}

#[test]
fn subscribe_raw_item_succeeds() {
    let reg = ComputeRegistry::new();
    let (tx, _rx) = bounded::<HashMap<String, f64>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
    let result = service.subscribe(key, Duration::from_millis(100), None);
    assert!(result.is_ok());
}

#[test]
fn end_to_end_calc_data_flow() {
    let mut reg = ComputeRegistry::new();
    reg.register_calc_realtime(Box::new(TestSpeedItem)).unwrap();

    let (data_tx, data_rx) = bounded::<HashMap<String, f64>>(10);
    let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(data_tx)));

    let key = ItemKey::parse("calc:test_speed").unwrap();
    service.subscribe(key.clone(), Duration::from_nanos(1), None).unwrap();

    frame_tx.send(Arc::new(make_frame(100.0))).unwrap();
    drop(frame_tx);
    let (_cmd_tx, cmd_rx) = bounded::<DashboardCommand>(1);
    service.run(frame_rx, cmd_rx);

    let data = data_rx.recv().unwrap();
    let val = data[&key.to_string()];
    assert!((val - 27.7777).abs() < 0.01);
}

#[test]
fn end_to_end_raw_item_flow() {
    let reg = ComputeRegistry::new();
    let (data_tx, data_rx) = bounded::<HashMap<String, f64>>(10);
    let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(data_tx)));

    let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
    service.subscribe(key.clone(), Duration::from_nanos(1), None).unwrap();

    frame_tx.send(Arc::new(make_frame(100.0))).unwrap();
    drop(frame_tx);
    let (_cmd_tx, cmd_rx) = bounded::<DashboardCommand>(1);
    service.run(frame_rx, cmd_rx);

    let data = data_rx.recv().unwrap();
    let val = data[&key.to_string()];
    assert!((val - 100.0).abs() < 0.01);
}

#[test]
fn unsubscribe_removes_item() {
    let reg = ComputeRegistry::new();
    let (tx, _rx) = bounded::<HashMap<String, f64>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
    service.subscribe(key.clone(), Duration::from_millis(100), None).unwrap();
    assert!(service.is_subscribed(&key));

    service.unsubscribe(&key);
    assert!(!service.is_subscribed(&key));
}
