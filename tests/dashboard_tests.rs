//! Dashboard 数据服务集成测试
//!
//! 测试 DashboardService 的公共 API：订阅验证、数据流端到端。

use module_live_telemetry::{
    TelemetryFrame,
    compute::ComputeRegistry,
    compute::items::SpeedMps,
    dashboard::service::DashboardService,
    dashboard::sink::ChannelSink,
    types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    },
};
use crossbeam_channel::bounded;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

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

#[test]
fn subscribe_unknown_item_returns_error() {
    let reg = ComputeRegistry::new();
    let (tx, _rx) = bounded::<HashMap<String, f64>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let result = service.subscribe("nonexistent".into(), Duration::from_millis(100), None);
    assert!(result.is_err());
    assert_eq!(service.subscription_count(), 0);
}

#[test]
fn subscribe_registered_item_succeeds() {
    let mut reg = ComputeRegistry::new();
    reg.register_realtime(Box::new(SpeedMps));
    let (tx, _rx) = bounded::<HashMap<String, f64>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(tx)));

    let result = service.subscribe("speed_mps".into(), Duration::from_millis(100), None);
    assert!(result.is_ok());
    assert!(service.is_subscribed("speed_mps"));
}

#[test]
fn end_to_end_data_flow() {
    let mut reg = ComputeRegistry::new();
    reg.register_realtime(Box::new(SpeedMps));

    let (data_tx, data_rx) = bounded::<HashMap<String, f64>>(10);
    let (frame_tx, frame_rx) = bounded::<Arc<TelemetryFrame>>(10);
    let mut service = DashboardService::new(reg, Box::new(ChannelSink::new(data_tx)));

    // Subscribe with zero interval so it triggers immediately
    service.subscribe("speed_mps".into(), Duration::from_nanos(1), None).unwrap();

    // Send a frame through
    frame_tx.send(Arc::new(make_frame(100.0))).unwrap();
    drop(frame_tx);

    service.run(frame_rx);

    // Should receive computed speed_mps
    let result = data_rx.try_recv().unwrap();
    assert!((result.get("speed_mps").copied().unwrap_or(0.0) - 27.7777).abs() < 0.1);
}

