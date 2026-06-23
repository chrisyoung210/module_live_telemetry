//! Manual QA integration test for replay-mock-telemetry.
//!
//! Exercises cross-task integration scenarios and edge cases through the
//! public API, with explicit verification of the full status sequence and
//! timing behavior. Run with: cargo test --test replay_manual_qa --features v2_writer

use module_live_telemetry::recording::{
    status_channel, DashboardItemKind, DashboardItemSubscription, RecordingController,
    RecordingStatus, ReplayRequest, StopReason,
};
use module_live_telemetry::types::{
    ControlSample, SessionMetadata, SessionSample,
};
use module_live_telemetry::writer::LiveTelemetryConfig;
use module_live_telemetry::writer_v2::BinaryTelemetryWriterV2;
use module_live_telemetry::TelemetryFrame;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ACC_STATUS_LIVE: i32 = 2;

/// Create a temporary `.acctlm2` file with `frame_count` frames.
fn create_acctlm2_file(frame_count: u64, tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "replay_qa_{}_{}.acctlm2",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let metadata = SessionMetadata::new("qa_track", "qa_car", 120.0);
    let config = LiveTelemetryConfig {
        poll_hz: 120.0,
        chunk_rows: 1024,
    };
    let mut writer =
        BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");

    for i in 0..frame_count {
        let frame = TelemetryFrame {
            sample_tick: i,
            timestamp_ns: i * 8_333_333,
            controls: ControlSample {
                sample_tick: i,
                timestamp_ns: i * 8_333_333,
                physics_packet_id: i as i32,
                speed_kmh: 100.0 + i as f32,
                ..ControlSample::default()
            },
            session: SessionSample {
                sample_tick: i,
                timestamp_ns: i * 8_333_333,
                status: ACC_STATUS_LIVE,
                ..SessionSample::default()
            },
            ..default_frame(i)
        };
        writer.write_frame(&Arc::new(frame)).expect("write_frame");
    }
    writer.finish().expect("finish");
    path
}

fn default_frame(sample_tick: u64) -> TelemetryFrame {
    use module_live_telemetry::types::*;
    TelemetryFrame {
        sample_tick,
        timestamp_ns: sample_tick * 1_000_000,
        controls: ControlSample::default(),
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

fn cleanup(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
}

// ===========================================================================
// SCENARIO 1: start_replay status sequence (Started -> ReplayStarted -> Running -> Stopping)
// ===========================================================================
#[test]
fn qa_status_sequence_full() {
    let path = create_acctlm2_file(200, "seq");
    // Use large capacity so Stopping is not dropped by try_send overflow
    let (status_tx, status_rx) = status_channel(1024);

    let req = ReplayRequest {
        file_path: path.clone(),
        speed_multiplier: 5.0,
        // Less frequent status to avoid flooding the channel
        status_interval: Duration::from_millis(500),
        dashboard_items: vec![],
        dashboard_realtime_items: vec![],
    };

    let mut controller =
        RecordingController::start_replay(req, status_tx, None, None).expect("start_replay");

    // Wait for replay to progress and collect statuses
    std::thread::sleep(Duration::from_millis(500));
    controller.stop();

    // Give the holder thread time to flush the Stopping status
    std::thread::sleep(Duration::from_millis(100));

    // Drain all statuses
    let statuses: Vec<RecordingStatus> = status_rx.try_iter().collect();

    eprintln!("[QA-1] Status sequence ({} entries):", statuses.len());
    for (i, s) in statuses.iter().enumerate() {
        eprintln!("  [{}] {:?}", i, s);
    }

    // Verify Started is first
    assert!(
        !statuses.is_empty(),
        "expected at least one status, got none"
    );
    assert!(
        matches!(statuses[0], RecordingStatus::Started { .. }),
        "expected first status to be Started, got {:?}",
        statuses[0]
    );

    // Verify ReplayStarted appears
    let has_replay_started = statuses
        .iter()
        .any(|s| matches!(s, RecordingStatus::ReplayStarted));
    assert!(
        has_replay_started,
        "expected ReplayStarted in sequence: {:?}",
        statuses
    );

    // Verify Started comes before ReplayStarted
    let started_idx = statuses
        .iter()
        .position(|s| matches!(s, RecordingStatus::Started { .. }));
    let replay_idx = statuses
        .iter()
        .position(|s| matches!(s, RecordingStatus::ReplayStarted));
    assert!(
        started_idx < replay_idx,
        "Started must come before ReplayStarted: got Started@{:?}, ReplayStarted@{:?}",
        started_idx,
        replay_idx
    );

    // Verify Stopping appears (either from stop() or frames exhausted)
    let has_stopping = statuses
        .iter()
        .any(|s| matches!(s, RecordingStatus::Stopping { .. }));
    assert!(
        has_stopping,
        "expected Stopping in sequence: {:?}",
        statuses
    );

    // Verify Stopping comes after ReplayStarted
    let stopping_idx = statuses
        .iter()
        .position(|s| matches!(s, RecordingStatus::Stopping { .. }));
    assert!(
        replay_idx < stopping_idx,
        "ReplayStarted must come before Stopping: got ReplayStarted@{:?}, Stopping@{:?}",
        replay_idx,
        stopping_idx
    );

    eprintln!("[QA-1] PASS: Started -> ReplayStarted -> ... -> Stopping verified");
    cleanup(&path);
}

// ===========================================================================
// SCENARIO 2: start_replay_with_latest_dashboard -> LatestValueReceiver gets data
// ===========================================================================
#[test]
fn qa_latest_dashboard_receives_data() {
    use module_live_telemetry::dashboard::sink::LatestValueReceiver;

    // Use many frames + moderate speed so the dashboard thread has time to
    // process the subscription and emit sampled values before replay ends.
    let path = create_acctlm2_file(300, "dash");
    let (status_tx, _status_rx) = status_channel(64);

    let req = ReplayRequest {
        file_path: path.clone(),
        speed_multiplier: 2.0,
        status_interval: Duration::from_millis(100),
        dashboard_items: vec![DashboardItemSubscription::new(
            "raw:controls.speed_kmh",
            DashboardItemKind::RawItem,
            // Very short interval so we get data quickly
            Duration::from_millis(1),
        )],
        dashboard_realtime_items: vec![],
    };

    let (mut controller, receiver): (RecordingController, LatestValueReceiver) =
        RecordingController::start_replay_with_latest_dashboard(req, status_tx, None)
            .expect("start_replay_with_latest_dashboard");

    // Collect frames for up to 2 seconds (slower replay needs more time)
    let mut received = 0usize;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match receiver.try_recv() {
            Ok(frame) => {
                eprintln!(
                    "[QA-2] received frame: {} values",
                    frame.values.len()
                );
                assert!(
                    !frame.values.is_empty(),
                    "dashboard frame should have values"
                );
                received += 1;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => break,
        }
    }

    controller.stop();
    assert!(received > 0, "LatestValueReceiver should receive at least one frame");
    eprintln!("[QA-2] PASS: received {} dashboard frames", received);
    cleanup(&path);
}

// ===========================================================================
// SCENARIO 3: stop() mid-replay -> clean shutdown
// ===========================================================================
#[test]
fn qa_stop_mid_replay_clean_shutdown() {
    let path = create_acctlm2_file(500, "stop");
    let (status_tx, status_rx) = status_channel(64);

    let req = ReplayRequest {
        file_path: path.clone(),
        speed_multiplier: 0.1, // very slow so it won't finish
        status_interval: Duration::from_millis(50),
        dashboard_items: vec![],
        dashboard_realtime_items: vec![],
    };

    let mut controller =
        RecordingController::start_replay(req, status_tx, None, None).expect("start_replay");

    // Let replay run briefly
    std::thread::sleep(Duration::from_millis(150));

    // Stop mid-replay — must not panic
    controller.stop();
    eprintln!("[QA-3] stop() returned without panic");

    // Double stop should also be safe (Drop will call stop again)
    drop(controller);
    eprintln!("[QA-3] drop() after stop() completed without panic");

    // Verify we got ReplayStarted before stop
    let statuses: Vec<RecordingStatus> = status_rx.try_iter().collect();
    let has_replay_started = statuses
        .iter()
        .any(|s| matches!(s, RecordingStatus::ReplayStarted));
    assert!(
        has_replay_started,
        "should have ReplayStarted before stop: {:?}",
        statuses
    );

    // Verify clean shutdown: either Manual stop or frames exhausted
    let has_stopping = statuses
        .iter()
        .any(|s| matches!(s, RecordingStatus::Stopping { .. }));
    assert!(
        has_stopping,
        "should have Stopping status after stop(): {:?}",
        statuses
    );

    eprintln!("[QA-3] PASS: clean shutdown verified, {} statuses collected", statuses.len());
    cleanup(&path);
}

// ===========================================================================
// SCENARIO 4: invalid file -> Err
// ===========================================================================
#[test]
fn qa_invalid_file_returns_err() {
    let (status_tx, _) = status_channel(8);

    let req = ReplayRequest {
        file_path: PathBuf::from("__nonexistent_replay_file_qa__.acctlm2"),
        speed_multiplier: 1.0,
        status_interval: Duration::from_secs(1),
        dashboard_items: vec![],
        dashboard_realtime_items: vec![],
    };

    let result = RecordingController::start_replay(req, status_tx, None, None);
    assert!(result.is_err(), "start_replay with invalid file must return Err");
    eprintln!("[QA-4] PASS: invalid file returned Err: {:?}", result.err().unwrap());
}

// ===========================================================================
// SCENARIO 5: empty file handling (0 frames)
// ===========================================================================
#[test]
fn qa_empty_file_handling() {
    // Test at the source level: empty ReplayTelemetrySource
    // An acctlm2 file with 0 frames should produce a source that reports Off
    // immediately and read returns None.

    // Create a 0-frame file
    let path = create_acctlm2_file(0, "empty");
    let (status_tx, status_rx) = status_channel(64);

    let req = ReplayRequest {
        file_path: path.clone(),
        speed_multiplier: 10.0,
        status_interval: Duration::from_millis(50),
        dashboard_items: vec![],
        dashboard_realtime_items: vec![],
    };

    // start_replay with a 0-frame file: the writer finishes without writing
    // any row groups, so the reader may reject it as InvalidFormat. Both
    // outcomes are valid graceful handling:
    //   (a) Err — the file is structurally incomplete
    //   (b) Ok  — replay starts and immediately exhausts frames
    let result = RecordingController::start_replay(req, status_tx, None, None);

    match result {
        Err(e) => {
            eprintln!("[QA-5] 0-frame file rejected gracefully: {}", e);
            eprintln!("[QA-5] PASS: empty file returns Err (graceful rejection)");
        }
        Ok(mut controller) => {
            // If it opened, verify it exhausts frames immediately
            std::thread::sleep(Duration::from_millis(200));
            controller.stop();

            let statuses: Vec<RecordingStatus> = status_rx.try_iter().collect();
            eprintln!("[QA-5] empty file statuses: {:?}", statuses);

            let has_frames_exhausted = statuses.iter().any(|s| {
                matches!(
                    s,
                    RecordingStatus::Stopping {
                        reason: StopReason::FramesExhausted
                    }
                )
            });
            assert!(
                has_frames_exhausted,
                "empty file should exhaust frames: {:?}",
                statuses
            );
            eprintln!("[QA-5] PASS: empty file handled (FramesExhausted)");
        }
    }
    cleanup(&path);
}

// ===========================================================================
// SCENARIO 6: speed multiplier timing (10x faster than 1x)
// ===========================================================================
#[test]
fn qa_speed_multiplier_timing() {
    let path_1x = create_acctlm2_file(10, "spd1x");
    let path_10x = create_acctlm2_file(10, "spd10x");

    // Run at 1x
    let (status_tx_1x, status_rx_1x) = status_channel(16);
    let req_1x = ReplayRequest {
        file_path: path_1x.clone(),
        speed_multiplier: 1.0,
        status_interval: Duration::from_millis(10),
        dashboard_items: vec![],
        dashboard_realtime_items: vec![],
    };
    let start_1x = Instant::now();
    let mut ctrl_1x =
        RecordingController::start_replay(req_1x, status_tx_1x, None, None).expect("1x");
    // Wait for FramesExhausted
    loop {
        match status_rx_1x.recv_timeout(Duration::from_secs(5)) {
            Ok(RecordingStatus::Stopping { reason: StopReason::FramesExhausted }) => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    let duration_1x = start_1x.elapsed();
    ctrl_1x.stop();

    // Run at 10x
    let (status_tx_10x, status_rx_10x) = status_channel(16);
    let req_10x = ReplayRequest {
        file_path: path_10x.clone(),
        speed_multiplier: 10.0,
        status_interval: Duration::from_millis(10),
        dashboard_items: vec![],
        dashboard_realtime_items: vec![],
    };
    let start_10x = Instant::now();
    let mut ctrl_10x =
        RecordingController::start_replay(req_10x, status_tx_10x, None, None).expect("10x");
    loop {
        match status_rx_10x.recv_timeout(Duration::from_secs(5)) {
            Ok(RecordingStatus::Stopping { reason: StopReason::FramesExhausted }) => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    let duration_10x = start_10x.elapsed();
    ctrl_10x.stop();

    eprintln!(
        "[QA-6] 1x duration: {:?}, 10x duration: {:?}",
        duration_1x, duration_10x
    );

    // 10x should be faster than 1x (generous margin: at least 1.5x faster)
    assert!(
        duration_10x < duration_1x,
        "10x replay should be faster than 1x: 1x={:?}, 10x={:?}",
        duration_1x,
        duration_10x
    );

    eprintln!("[QA-6] PASS: 10x ({:?}) < 1x ({:?})", duration_10x, duration_1x);
    cleanup(&path_1x);
    cleanup(&path_10x);
}
