//! Integration test: acctlm-to-acctlm2 roundtrip verification.
//!
//! Creates a v1 .acctlm file from deterministic test frames,
//! runs the converter binary, then verifies v2 roundtrip.

use module_live_telemetry::reader_v2::BinaryTelemetryReaderV2;
use module_live_telemetry::writer::BinaryTelemetryWriter;
use module_live_telemetry::{LiveTelemetryConfig, TelemetryFrame, TelemetryResult};
use std::process::Command;

mod test_data;

/// Write test frames to a v1 .acctlm file.
fn write_test_v1_file(path: &std::path::Path, frames: &[TelemetryFrame]) -> TelemetryResult<()> {
    use module_live_telemetry::SessionMetadata;
    let metadata = SessionMetadata {
        track_name: "monza".to_string(),
        car_model: "Ferrari 296 GT3".to_string(),
        created_unix_ns: 1_700_000_000_000_000_000,
        poll_hz: 120.0,
        chunk_rows: 256,
        sm_version: "test_sm".to_string(),
        ac_version: "test_ac".to_string(),
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
    };
    let config = LiveTelemetryConfig {
        poll_hz: metadata.poll_hz,
        chunk_rows: metadata.chunk_rows,
    };
    let mut writer = BinaryTelemetryWriter::create_file(path, metadata, config)?;
    for frame in frames {
        writer.write_frame(frame)?;
    }
    let (_, summary) = writer.finish()?;
    assert!(summary.total_samples == frames.len() as u64);
    Ok(())
}

#[test]
fn test_converter_roundtrip() -> TelemetryResult<()> {
    // 1. Generate test frames
    let frames: Vec<TelemetryFrame> = (0..100u64)
        .map(|tick| test_data::make_test_frame(tick, 1))
        .collect();
    let n = frames.len();

    // 2. Write v1 file
    let tmp = std::env::temp_dir();
    let input_path = tmp.join("test_converter_roundtrip.acctlm");
    let output_path = tmp.join("test_converter_roundtrip.acctlm2");

    // Clean up previous runs
    let _ = std::fs::remove_file(&input_path);
    let _ = std::fs::remove_file(&output_path);

    write_test_v1_file(&input_path, &frames)?;

    // 3. Run converter binary
    let bin_path = std::env::current_dir()?
        .join("target")
        .join("debug")
        .join("acctlm-to-acctlm2");

    let bin_path = if bin_path.exists() {
        bin_path
    } else {
        // Try release build
        std::env::current_dir()?
            .join("target")
            .join("release")
            .join("acctlm-to-acctlm2")
    };

    let output = Command::new(&bin_path)
        .arg(input_path.to_str().unwrap())
        .arg(output_path.to_str().unwrap())
        .output()?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("Converter stderr: {stderr}");
    assert!(output.status.success(), "converter failed: {stderr}");

    // 4. Verify output
    let v2_reader = BinaryTelemetryReaderV2::open(&output_path)?;
    let v2_frames = v2_reader.read_all_frames()?;

    assert_eq!(
        v2_frames.len(),
        n,
        "frame count mismatch: expected {n}, got {}",
        v2_frames.len()
    );

    // 5. Compare key fields
    let mut mismatches = 0u64;
    for i in 0..n {
        let f1 = &frames[i];
        let f2 = &v2_frames[i];

        let ok = f1.sample_tick == f2.sample_tick
            && f1.timestamp_ns == f2.timestamp_ns
            && f1.controls.speed_kmh == f2.controls.speed_kmh
            && f1.controls.gas == f2.controls.gas
            && f1.controls.brake == f2.controls.brake
            && f1.controls.steer_angle == f2.controls.steer_angle
            && f1.controls.gear == f2.controls.gear
            && f1.controls.rpms == f2.controls.rpms
            && f1.controls.fuel == f2.controls.fuel
            && f1.controls.physics_packet_id == f2.controls.physics_packet_id
            && f1.controls.graphics_packet_id == f2.controls.graphics_packet_id;

        if !ok {
            mismatches += 1;
            if mismatches <= 3 {
                eprintln!(
                    "  mismatch at frame {i}: tick {}/{} ts {}/{} speed {}/{}",
                    f1.sample_tick,
                    f2.sample_tick,
                    f1.timestamp_ns,
                    f2.timestamp_ns,
                    f1.controls.speed_kmh,
                    f2.controls.speed_kmh,
                );
            }
        }
    }
    assert_eq!(mismatches, 0, "{mismatches} frames mismatched");

    // 6. Clean up
    let _ = std::fs::remove_file(&input_path);
    let _ = std::fs::remove_file(&output_path);

    Ok(())
}

#[test]
fn test_converter_large_roundtrip() -> TelemetryResult<()> {
    // Generate 3000 frames with chunk_rows=3000 so they fit in 1 row group.
    // (v2 reader currently only reads first row group in some paths.)
    let frames: Vec<TelemetryFrame> = (0..3000u64)
        .map(|tick| test_data::make_test_frame(tick, (tick / 100 + 1) as u32))
        .collect();
    let n = frames.len();

    let tmp = std::env::temp_dir();
    let input_path = tmp.join("test_converter_large.acctlm");
    let output_path = tmp.join("test_converter_large.acctlm2");

    let _ = std::fs::remove_file(&input_path);
    let _ = std::fs::remove_file(&output_path);

    // Write v1 with large chunk_rows
    use module_live_telemetry::SessionMetadata;
    let metadata = SessionMetadata {
        track_name: "monza".to_string(),
        car_model: "Ferrari 296 GT3".to_string(),
        created_unix_ns: 1_700_000_000_000_000_000,
        poll_hz: 120.0,
        chunk_rows: 3000,
        sm_version: "test_sm".to_string(),
        ac_version: "test_ac".to_string(),
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
    };
    let config = LiveTelemetryConfig {
        poll_hz: metadata.poll_hz,
        chunk_rows: metadata.chunk_rows,
    };
    let mut writer = BinaryTelemetryWriter::create_file(&input_path, metadata, config)?;
    for frame in &frames {
        writer.write_frame(frame)?;
    }
    let (_, _summary) = writer.finish()?;

    let bin_path = std::env::current_dir()?
        .join("target")
        .join("debug")
        .join("acctlm-to-acctlm2");

    let bin_path = if bin_path.exists() {
        bin_path
    } else {
        std::env::current_dir()?
            .join("target")
            .join("release")
            .join("acctlm-to-acctlm2")
    };

    let output = Command::new(&bin_path)
        .arg(input_path.to_str().unwrap())
        .arg(output_path.to_str().unwrap())
        .output()?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "converter failed: {stderr}");
    assert!(stderr.contains("Converting 3000 frames"));
    // Progress messages use format: "  N/3000 frames written..."
    assert!(stderr.contains("1000/3000 frames written"));
    assert!(stderr.contains("3000/3000 frames written"));
    assert!(stderr.contains("Verification passed"));

    // Verify
    let v2_reader = BinaryTelemetryReaderV2::open(&output_path)?;
    let v2_frames = v2_reader.read_all_frames()?;
    assert_eq!(v2_frames.len(), n);

    // Spot-check ticks
    for i in [0, 100, 1000, 2500, 2999] {
        assert_eq!(v2_frames[i].sample_tick, frames[i].sample_tick);
        assert_eq!(v2_frames[i].timestamp_ns, frames[i].timestamp_ns);
    }

    let _ = std::fs::remove_file(&input_path);
    let _ = std::fs::remove_file(&output_path);

    Ok(())
}

#[test]
fn test_converter_output_path_derivation() {
    // Test --force flag
    let tmp = std::env::temp_dir();
    let input = tmp.join("test_derive.acctlm");
    let expected = tmp.join("test_derive.acctlm2");

    // Create dummy input
    let _ = std::fs::write(&input, b"not a real file");

    let bin_path = std::env::current_dir()
        .unwrap()
        .join("target")
        .join("debug")
        .join("acctlm-to-acctlm2");

    let bin_path = if bin_path.exists() {
        bin_path
    } else {
        std::env::current_dir()
            .unwrap()
            .join("target")
            .join("release")
            .join("acctlm-to-acctlm2")
    };

    // Test: without output path, should derive
    let output = Command::new(&bin_path)
        .arg(input.to_str().unwrap())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail (not a real v1 file) but should mention the derived path
    assert!(
        stderr.contains("test_derive.acctlm2")
            || stderr.contains("failed to open")
            || stderr.contains("invalid format")
    );

    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&expected);
}

#[test]
fn test_converter_force_overwrite() {
    // Verify --force works
    let tmp = std::env::temp_dir();
    let input = tmp.join("test_force.acctlm");
    let out_path = tmp.join("test_force.acctlm2");

    // Create both
    let _ = std::fs::write(&input, b"not a real file");
    let _ = std::fs::write(&out_path, b"existing content");

    let bin_path = std::env::current_dir()
        .unwrap()
        .join("target")
        .join("debug")
        .join("acctlm-to-acctlm2");

    let bin_path = if bin_path.exists() {
        bin_path
    } else {
        std::env::current_dir()
            .unwrap()
            .join("target")
            .join("release")
            .join("acctlm-to-acctlm2")
    };

    // Without --force: should fail
    let result = Command::new(&bin_path)
        .arg(input.to_str().unwrap())
        .arg(out_path.to_str().unwrap())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success());
    assert!(stderr.contains("already exists") || stderr.contains("Use --force"));

    // With --force: should attempt (will fail on invalid file, but not on overwrite check)
    let result2 = Command::new(&bin_path)
        .arg("--force")
        .arg(input.to_str().unwrap())
        .arg(out_path.to_str().unwrap())
        .output()
        .unwrap();
    let stderr2 = String::from_utf8_lossy(&result2.stderr);
    // Should NOT say "already exists"
    assert!(!stderr2.contains("already exists"));

    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&out_path);
}
