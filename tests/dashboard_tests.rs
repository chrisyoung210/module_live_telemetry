//! Dashboard 数据服务集成测试
//!
//! 测试 DashboardService 的公共 API：订阅验证、数据流端到端。
//! 使用本地定义的测试 item 替代已移除的 SpeedMps。

use crossbeam_channel::bounded;
use module_live_telemetry::{
    compute::{
        items::{create_prev_sector_items, create_sector_best_items, RealtimeComputeItem},
        ComputeContext, ComputeRegistry, ComputeResult,
    },
    dashboard::service::{DashboardCommand, DashboardService},
    dashboard::sink::ChannelSink,
    item_key::ItemKey,
    recording::DashboardValuesFrame,
    types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
        PowertrainSample, SessionSample, TimingSample, TyreSample,
    },
    TelemetryFrame,
};
use std::sync::Arc;
use std::time::Duration;

/// 本地测试用计算项：km/h → m/s
struct TestSpeedItem;
impl RealtimeComputeItem for TestSpeedItem {
    fn name(&self) -> &str {
        "test_speed"
    }
    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        Ok(ctx.current_frame.controls.speed_kmh as f64 / 3.6)
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
fn subscribe_unknown_item_returns_error() {
    let reg = ComputeRegistry::new();
    let (tx, _rx) = bounded::<DashboardValuesFrame>(10);
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
    let (tx, _rx) = bounded::<DashboardValuesFrame>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let key = ItemKey::parse("calc:test_speed").unwrap();
    let result = service.subscribe(key.clone(), Duration::from_millis(100), None);
    assert!(result.is_ok());
    assert!(service.is_subscribed(&key));
}

#[test]
fn subscribe_raw_item_succeeds() {
    let reg = ComputeRegistry::new();
    let (tx, _rx) = bounded::<DashboardValuesFrame>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
    let result = service.subscribe(key, Duration::from_millis(100), None);
    assert!(result.is_ok());
}

#[test]
fn end_to_end_calc_data_flow() {
    let mut reg = ComputeRegistry::new();
    reg.register_calc_realtime(Box::new(TestSpeedItem)).unwrap();

    let (data_tx, data_rx) = bounded::<DashboardValuesFrame>(10);
    let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(data_tx)));

    let key = ItemKey::parse("calc:test_speed").unwrap();
    service
        .subscribe(key.clone(), Duration::from_nanos(1), None)
        .unwrap();

    frame_tx.send(Arc::new(make_frame(100.0))).unwrap();
    drop(frame_tx);
    let (_cmd_tx, cmd_rx) = bounded::<DashboardCommand>(1);
    service.run(frame_rx, cmd_rx);

    let data = data_rx.recv().unwrap();
    let val = data.values[&key.to_string()];
    assert!((val - 27.7777).abs() < 0.01);
}

#[test]
fn end_to_end_raw_item_flow() {
    let reg = ComputeRegistry::new();
    let (data_tx, data_rx) = bounded::<DashboardValuesFrame>(10);
    let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(data_tx)));

    let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
    service
        .subscribe(key.clone(), Duration::from_nanos(1), None)
        .unwrap();

    frame_tx.send(Arc::new(make_frame(100.0))).unwrap();
    drop(frame_tx);
    let (_cmd_tx, cmd_rx) = bounded::<DashboardCommand>(1);
    service.run(frame_rx, cmd_rx);

    let data = data_rx.recv().unwrap();
    let val = data.values[&key.to_string()];
    assert!((val - 100.0).abs() < 0.01);
}

#[test]
fn unsubscribe_removes_item() {
    let reg = ComputeRegistry::new();
    let (tx, _rx) = bounded::<DashboardValuesFrame>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
    service
        .subscribe(key.clone(), Duration::from_millis(100), None)
        .unwrap();
    assert!(service.is_subscribed(&key));

    service.unsubscribe(&key);
    assert!(!service.is_subscribed(&key));
}

#[test]
fn test_sector_items_end_to_end() {
    let mut reg = ComputeRegistry::new();

    // Register prev sector items (pair shares same SectorState)
    let (prev_time, prev_num) = create_prev_sector_items();
    reg.register_calc_realtime(Box::new(prev_time)).unwrap();
    reg.register_calc_realtime(Box::new(prev_num)).unwrap();

    // Register all 3 sector best items (share same SectorBestState)
    let (s0, s1, s2) = create_sector_best_items();
    reg.register_calc_realtime(Box::new(s0)).unwrap();
    reg.register_calc_realtime(Box::new(s1)).unwrap();
    reg.register_calc_realtime(Box::new(s2)).unwrap();

    let (data_tx, data_rx) = bounded::<DashboardValuesFrame>(10);
    let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(data_tx)));

    let key = ItemKey::parse("calc:prev_sector_time").unwrap();
    service
        .subscribe(key.clone(), Duration::from_nanos(1), None)
        .unwrap();

    // Frame 1: sector 0, no transition yet (last_seen_sector = -1)
    frame_tx
        .send(Arc::new(make_sector_frame(0, 1, 12000, 1)))
        .unwrap();
    // Frame 2: sector 1, transition 0→1, captures last_sector_time=25000
    frame_tx
        .send(Arc::new(make_sector_frame(1, 1, 25000, 1)))
        .unwrap();
    drop(frame_tx);

    let (_cmd_tx, cmd_rx) = bounded::<DashboardCommand>(1);
    service.run(frame_rx, cmd_rx);

    // Collect all output — expect 2 messages (one per frame)
    let mut last_data = data_rx.recv().unwrap(); // first frame output
    if let Ok(data) = data_rx.try_recv() {
        last_data = data; // second frame output (with transition)
    }

    let val = last_data.values[&key.to_string()];
    assert!((val - 25000.0).abs() < 0.01);
}
