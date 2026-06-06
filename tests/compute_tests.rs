//! 计算项系统集成测试
//!
//! 测试 ComputeRegistry 的公共 API。

use module_live_telemetry::{
    TelemetryFrame,
    compute::{ComputeRegistry, ComputeError, RealtimeComputeRequest},
    compute::items::SpeedMps,
    compute::context::ReferenceSource,
    types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    },
};
use std::collections::HashMap;
use std::path::PathBuf;

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
fn compute_realtime_item_returns_correct_value() {
    let mut registry = ComputeRegistry::new();
    registry.register_realtime(Box::new(SpeedMps));

    let frame = make_frame(100.0);
    let values = HashMap::new();
    let request = RealtimeComputeRequest {
        current_frame: &frame,
        computed_values: &values,
        reference_lap: None,
        reference_source: None,
    };

    let result = registry.compute_realtime("speed_mps", &request).unwrap();
    assert!((result - 27.7777).abs() < 0.01);
}

#[test]
fn compute_realtime_item_not_found() {
    let mut registry = ComputeRegistry::new();
    let frame = make_frame(100.0);
    let values = HashMap::new();
    let request = RealtimeComputeRequest {
        current_frame: &frame,
        computed_values: &values,
        reference_lap: None,
        reference_source: None,
    };

    let result = registry.compute_realtime("nonexistent", &request);
    assert!(matches!(result, Err(ComputeError::ItemNotFound(_))));
}

#[test]
fn reference_cache_evicts_on_overflow() {
    let mut registry = ComputeRegistry::new();
    let frame = make_frame(0.0);

    // Fill cache beyond MAX_CACHE_ENTRIES (4)
    for i in 0..6 {
        registry.cache_reference_lap(
            ReferenceSource {
                file_path: PathBuf::from(format!("test_{i}.acctlm")),
                lap_number: 1,
            },
            vec![frame.clone()],
        );
    }

    // Should still hold entries (old ones evicted silently)
    let source = ReferenceSource {
        file_path: PathBuf::from("test_5.acctlm"),
        lap_number: 1,
    };
    assert!(registry.get_reference_lap(&source).is_some());
}
