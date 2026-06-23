//! Performance benchmarks comparing v1 vs v2 acctlm formats.
//!
//! Three benchmarks:
//! - `write`    — v1 writer vs v2 writer (10K frames)
//! - `read`     — v1 full read vs v2 full read (10K frames)
//! - `selective_read` — v2 selective group read vs v2 full read
//!
//! Frames are generated once with varied data to exercise the encoding paths.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::fs;

use module_live_telemetry::{
    any_reader::open_telemetry_file, BinaryTelemetryReaderV2, BinaryTelemetryWriter,
    BinaryTelemetryWriterV2, CarStateSample, ControlSample, EnvironmentSample, LiveTelemetryConfig,
    MotionSample, OtherCarsSample, PowertrainSample, SessionMetadata, SessionSample,
    TelemetryFrame, TimingSample, TyreSample,
};

const FRAME_COUNT: u64 = 10_000;
const CHUNK_ROWS: usize = 1024;

// ---------------------------------------------------------------------------
// Test data generator
// ---------------------------------------------------------------------------

fn make_metadata() -> SessionMetadata {
    SessionMetadata {
        track_name: "bench_track".into(),
        car_model: "bench_car".into(),
        created_unix_ns: 1_700_000_000_000_000_000,
        poll_hz: 120.0,
        chunk_rows: CHUNK_ROWS,
        sm_version: "bench".into(),
        ac_version: "bench".into(),
        number_of_sessions: 1,
        num_cars: 24,
        sector_count: 3,
        max_rpm: 9000,
        max_torque: 650.0,
        max_power: 700.0,
        max_fuel: 100.0,
        penalties_enabled: 1,
        raw_static_bytes: vec![0u8; 1024],
        session_type: Some(9),
    }
}

/// Generate a single varied frame seeded by tick (simplified from test_data).
fn make_frame(tick: u64) -> TelemetryFrame {
    let ns = tick * 8_333_333;
    let t = tick as f32;
    let lap = 1i32 + (tick / 100) as i32;
    let gear = 1 + (tick % 7) as i32;

    TelemetryFrame {
        sample_tick: tick,
        timestamp_ns: ns,
        controls: ControlSample {
            sample_tick: tick,
            timestamp_ns: ns,
            speed_kmh: 150.0 + t * 0.1,
            gas: ((tick % 100) as f32) / 100.0,
            brake: (((tick + 50) % 100) as f32) / 120.0,
            clutch: ((tick % 10) as f32) / 100.0,
            steer_angle: (t * 0.01).sin(),
            gear,
            rpms: (5000 + (tick % 4000)) as i32,
            fuel: (100.0 - t * 0.001).max(0.0),
            ..Default::default()
        },
        motion: MotionSample {
            sample_tick: tick,
            timestamp_ns: ns,
            velocity: [t * 0.5, t * 0.3, t * 0.1],
            heading: (t * 0.001).sin(),
            pitch: (t * 0.002).sin(),
            roll: (t * 0.003).sin(),
            ..Default::default()
        },
        tyres: TyreSample {
            sample_tick: tick,
            timestamp_ns: ns,
            wheel_load: [
                3000.0 + t * 0.1,
                3100.0 + t * 0.1,
                2900.0 + t * 0.1,
                3050.0 + t * 0.1,
            ],
            tyre_wear: [
                (t * 0.001) as f32,
                (t * 0.0011) as f32,
                (t * 0.0009) as f32,
                (t * 0.0012) as f32,
            ],
            ..Default::default()
        },
        powertrain: PowertrainSample {
            sample_tick: tick,
            timestamp_ns: ns,
            turbo_boost: 1.5 + (tick % 100) as f32 * 0.01,
            kers_charge: 50.0 + (tick % 50) as f32,
            ..Default::default()
        },
        session: SessionSample {
            sample_tick: tick,
            timestamp_ns: ns,
            status: 2,
            session: 1,
            session_index: 0,
            completed_laps: lap - 1,
            position: (1 + (tick % 24)) as i32,
            ..Default::default()
        },
        timing: TimingSample {
            sample_tick: tick,
            timestamp_ns: ns,
            i_current_time: (tick as i32) * 1000,
            i_last_time: (tick as i32) * 995,
            i_best_time: (tick as i32) * 990,
            i_split: (tick as i32) * 330,
            ..Default::default()
        },
        car_state: CarStateSample {
            sample_tick: tick,
            timestamp_ns: ns,
            cg_height: 0.35,
            brake_bias: 55.0 + t * 0.1,
            ..Default::default()
        },
        environment: EnvironmentSample {
            sample_tick: tick,
            timestamp_ns: ns,
            air_temp: 25.0 + (tick % 20) as f32,
            road_temp: 32.0 + (tick % 15) as f32,
            ..Default::default()
        },
        other_cars: OtherCarsSample {
            sample_tick: tick,
            timestamp_ns: ns,
            active_cars: (20 + (tick % 4)) as i32,
            ..Default::default()
        },
    }
}

fn generate_frames() -> Vec<TelemetryFrame> {
    (0..FRAME_COUNT).map(make_frame).collect()
}

fn temp_v1_path() -> std::path::PathBuf {
    std::env::temp_dir().join("bench_v1.acctlm")
}

fn temp_v2_path() -> std::path::PathBuf {
    std::env::temp_dir().join("bench_v2.acctlm2")
}

// ---------------------------------------------------------------------------
// Bench: write — v1 vs v2
// ---------------------------------------------------------------------------

fn bench_write(c: &mut Criterion) {
    let frames = generate_frames();
    let meta = make_metadata();
    let config = LiveTelemetryConfig::default();

    let mut group = c.benchmark_group("write");
    group.sample_size(10);

    group.bench_function("v1", |b| {
        b.iter(|| {
            let path = temp_v1_path();
            let mut w =
                BinaryTelemetryWriter::create_file(&path, meta.clone(), config.clone()).unwrap();
            for f in &frames {
                w.write_frame(f).unwrap();
            }
            let _ = w.finish().unwrap();
            let _ = fs::remove_file(&path);
        });
    });

    group.bench_function("v2", |b| {
        b.iter(|| {
            let path = temp_v2_path();
            let mut w =
                BinaryTelemetryWriterV2::create_file(&path, meta.clone(), config.clone()).unwrap();
            for f in &frames {
                w.write_frame(f).unwrap();
            }
            let _ = w.finish().unwrap();
            let _ = fs::remove_file(&path);
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Bench: read — v1 full vs v2 full
// ---------------------------------------------------------------------------

fn bench_read(c: &mut Criterion) {
    let frames = generate_frames();
    let meta = make_metadata();
    let config = LiveTelemetryConfig::default();

    let v1_path = temp_v1_path();
    let v2_path = temp_v2_path();

    // Pre-write files (not part of benchmark)
    {
        let mut w =
            BinaryTelemetryWriter::create_file(&v1_path, meta.clone(), config.clone()).unwrap();
        for f in &frames {
            w.write_frame(f).unwrap();
        }
        let _ = w.finish().unwrap();
    }
    {
        let mut w =
            BinaryTelemetryWriterV2::create_file(&v2_path, meta.clone(), config.clone()).unwrap();
        for f in &frames {
            w.write_frame(f).unwrap();
        }
        let _ = w.finish().unwrap();
    }

    let mut group = c.benchmark_group("read");
    group.sample_size(10);

    group.bench_function("v1", |b| {
        b.iter(|| {
            let reader = open_telemetry_file(&v1_path).unwrap();
            let frames = reader.read_all_frames().unwrap();
            black_box(frames);
        });
    });

    group.bench_function("v2", |b| {
        b.iter(|| {
            let reader = BinaryTelemetryReaderV2::open(&v2_path).unwrap();
            let frames = reader.read_all_frames().unwrap();
            black_box(frames);
        });
    });

    group.finish();

    let _ = fs::remove_file(&v1_path);
    let _ = fs::remove_file(&v2_path);
}

// ---------------------------------------------------------------------------
// Bench: selective_read — v2 group read vs v2 full read
// ---------------------------------------------------------------------------

fn bench_selective_read(c: &mut Criterion) {
    let frames = generate_frames();
    let meta = make_metadata();
    let config = LiveTelemetryConfig::default();

    let v2_path = temp_v2_path();

    // Pre-write v2 file
    {
        let mut w =
            BinaryTelemetryWriterV2::create_file(&v2_path, meta.clone(), config.clone()).unwrap();
        for f in &frames {
            w.write_frame(f).unwrap();
        }
        let _ = w.finish().unwrap();
    }

    let mut group = c.benchmark_group("selective_read");
    group.sample_size(10);

    group.bench_function("v2_full", |b| {
        b.iter(|| {
            let reader = BinaryTelemetryReaderV2::open(&v2_path).unwrap();
            let frames = reader.read_all_frames().unwrap();
            black_box(frames);
        });
    });

    group.bench_function("v2_driver_inputs", |b| {
        b.iter(|| {
            let reader = BinaryTelemetryReaderV2::open(&v2_path).unwrap();
            let controls = reader.read_all_controls_v2().unwrap();
            black_box(controls);
        });
    });

    group.finish();

    let _ = fs::remove_file(&v2_path);
}

criterion_group!(benches, bench_write, bench_read, bench_selective_read);
criterion_main!(benches);
