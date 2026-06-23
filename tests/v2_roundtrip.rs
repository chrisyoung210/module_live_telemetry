//! V2 format roundtrip tests.
//!
//! These tests verify write→read correctness of the acctlm2 binary format
//! using the v2 writer/reader API (`BinaryTelemetryWriterV2` /
//! `BinaryTelemetryReaderV2`).
//!
//! All v2 roundtrip tests are gated behind `#[cfg(feature = "v2_writer")]`
//! so they only compile when the writer/reader implementation exists.
//! They are also marked `#[ignore]` (RED phase) — they should not be
//! executed until the implementation is complete enough to pass them.
//!
//! One infrastructure test (always active) confirms that the test data
//! generator itself produces the expected number of frames.

/// Deterministic test data generation.
mod test_data;
use test_data::{make_test_metadata, make_test_session};

// =========================================================================
// Infrastructure test (always compiled, always passes)
// =========================================================================

/// Verify that the test data generator itself works correctly.
///
/// This test confirms that `make_test_session` produces the expected
/// number of frames with correct lap assignment and populated data.
/// It MUST pass unconditionally — if this fails, the data generator
/// is broken, not the v2 format.
#[test]
fn test_infrastructure_works() {
    // --- basic count ---
    let (_meta, frames) = make_test_session(3, 5);
    assert_eq!(frames.len(), 15, "3 laps × 5 fpl = 15 frames");

    // --- each frame has non-default data in every substructure ---
    for (i, f) in frames.iter().enumerate() {
        assert!(f.controls.speed_kmh > 0.0, "frame[{i}].speed_kmh populated");
        assert!(f.motion.velocity[0] >= 0.0, "frame[{i}].velocity populated");
        assert!(
            f.tyres.wheel_load[0] > 1000.0,
            "frame[{i}].wheel_load populated"
        );
        assert!(
            f.powertrain.turbo_boost > 1.0,
            "frame[{i}].turbo_boost populated"
        );
        assert_eq!(f.session.status, 2, "frame[{i}].status");
        assert!(
            f.timing.i_current_time >= 0,
            "frame[{i}].i_current_time populated"
        );
        assert!(
            f.car_state.cg_height > 0.3,
            "frame[{i}].cg_height populated"
        );
        assert!(
            f.environment.air_temp > 20.0,
            "frame[{i}].air_temp populated"
        );
        assert!(
            f.other_cars.active_cars > 0,
            "frame[{i}].active_cars populated"
        );
    }

    // --- lap boundaries ---
    let fpl = 5usize;
    let (_meta, frames) = make_test_session(3, fpl as u64);
    // Lap 1 → ticks 0..4, completed_laps = 0
    assert_eq!(frames[0].session.completed_laps, 0, "first frame lap 1");
    assert_eq!(
        frames[fpl - 1].session.completed_laps,
        0,
        "last frame lap 1"
    );
    // Lap 2 → ticks 5..9, completed_laps = 1
    assert_eq!(frames[fpl].session.completed_laps, 1, "first frame lap 2");
    assert_eq!(
        frames[2 * fpl - 1].session.completed_laps,
        1,
        "last frame lap 2"
    );
    // Lap 3 → ticks 10..14, completed_laps = 2
    assert_eq!(
        frames[2 * fpl].session.completed_laps,
        2,
        "first frame lap 3"
    );
    assert_eq!(
        frames[3 * fpl - 1].session.completed_laps,
        2,
        "last frame lap 3"
    );

    // --- metadata consistency ---
    let metadata = make_test_metadata("test_track", "test_car");
    assert_eq!(metadata.track_name, "test_track");
    assert_eq!(metadata.car_model, "test_car");
    assert!(metadata.poll_hz > 0.0);
}

// =========================================================================
// V2 roundtrip tests (RED phase)
// =========================================================================
//
// All tests below are gated behind `#[cfg(feature = "v2_writer")]` —
// they will NOT compile until the feature is activated (i.e., until
// the v2 writer/reader implementation is added to the crate).
//
// Each test is also marked `#[ignore]` so they do not run by default
// even when the feature is active.  Remove `#[ignore]` once the
// implementation is ready to be verified.

// ---------------------------------------------------------------------------
// Expected API shape
//
// The tests below use the following API, which must be provided by
// `src/writer_v2.rs` and `src/reader_v2.rs`:
//
// ```ignore
// BinaryTelemetryWriterV2::create_file(path, metadata, config) -> Result<Self>
// BinaryTelemetryWriterV2::write_frame(&mut self, frame) -> Result<()>
// BinaryTelemetryWriterV2::finish(self) -> Result<RecordingSummary>
//
// BinaryTelemetryReaderV2::open(path) -> Result<Self>
// BinaryTelemetryReaderV2::metadata(&self) -> &SessionMetadata
// BinaryTelemetryReaderV2::read_all_frames(&self) -> Result<Vec<TelemetryFrame>>
// BinaryTelemetryReaderV2::read_lap_frames(&self, lap_number: u32) -> Result<Vec<TelemetryFrame>>
// BinaryTelemetryReaderV2::read_group_frames(
//     &self,
//     groups: &[GroupId],
//     start_frame: Option<u64>,
//     end_frame: Option<u64>,
// ) -> Result<HashMap<GroupId, Vec<Vec<f64>>>>
// ```
// ---------------------------------------------------------------------------

#[cfg(feature = "v2_writer")]
mod v2_tests {
    use super::test_data::make_test_frame;
    use super::*;
    use module_live_telemetry::format_v2::GroupId;
    use module_live_telemetry::reader_v2::BinaryTelemetryReaderV2;
    use module_live_telemetry::writer::LiveTelemetryConfig;
    use module_live_telemetry::writer_v2::BinaryTelemetryWriterV2;
    use module_live_telemetry::{SessionMetadata, TelemetryFrame};
    use std::collections::HashMap;
    use std::path::Path;

    /// Create a temporary directory for test file I/O.
    ///
    /// The directory is rooted under the OS temp dir and cleaned up when the
    /// returned guard is dropped or explicitly by calling `cleanup()`.
    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{}_{}", label, std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("failed to create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn file_path(&self, name: &str) -> std::path::PathBuf {
            self.path.join(name)
        }

        fn cleanup(&self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            self.cleanup();
        }
    }

    // The writer and reader for v2 format.  These are expected to be
    // re-exported from the crate root when the `v2_writer` feature is
    // active.
    // use module_live_telemetry::BinaryTelemetryWriterV2;
    // use module_live_telemetry::BinaryTelemetryReaderV2;

    // =====================================================================
    // Helper — deep field-by-field comparison of two TelemetryFrames
    // =====================================================================

    /// Assert that two frames are identical across all substructures.
    fn assert_frames_equal(a: &TelemetryFrame, b: &TelemetryFrame, idx: usize) {
        // Top-level
        assert_eq!(a.sample_tick, b.sample_tick, "frame[{idx}].sample_tick");
        assert_eq!(a.timestamp_ns, b.timestamp_ns, "frame[{idx}].timestamp_ns");

        // Controls (DriverInputs group)
        assert_eq!(
            a.controls.speed_kmh, b.controls.speed_kmh,
            "frame[{idx}].controls.speed_kmh"
        );
        assert_eq!(a.controls.gas, b.controls.gas, "frame[{idx}].controls.gas");
        assert_eq!(
            a.controls.brake, b.controls.brake,
            "frame[{idx}].controls.brake"
        );
        assert_eq!(
            a.controls.clutch, b.controls.clutch,
            "frame[{idx}].controls.clutch"
        );
        assert_eq!(
            a.controls.steer_angle, b.controls.steer_angle,
            "frame[{idx}].controls.steer_angle"
        );
        assert_eq!(
            a.controls.gear, b.controls.gear,
            "frame[{idx}].controls.gear"
        );
        assert_eq!(
            a.controls.rpms, b.controls.rpms,
            "frame[{idx}].controls.rpms"
        );
        assert_eq!(
            a.controls.fuel, b.controls.fuel,
            "frame[{idx}].controls.fuel"
        );

        // Motion (VehicleDynamics group)
        assert_eq!(
            a.motion.velocity, b.motion.velocity,
            "frame[{idx}].motion.velocity"
        );
        assert_eq!(a.motion.acc_g, b.motion.acc_g, "frame[{idx}].motion.acc_g");
        assert_eq!(
            a.motion.heading, b.motion.heading,
            "frame[{idx}].motion.heading"
        );
        assert_eq!(a.motion.pitch, b.motion.pitch, "frame[{idx}].motion.pitch");
        assert_eq!(a.motion.roll, b.motion.roll, "frame[{idx}].motion.roll");

        // Tyres (Tyres group)
        assert_eq!(
            a.tyres.wheel_slip, b.tyres.wheel_slip,
            "frame[{idx}].tyres.wheel_slip"
        );
        assert_eq!(
            a.tyres.wheel_load, b.tyres.wheel_load,
            "frame[{idx}].tyres.wheel_load"
        );
        assert_eq!(
            a.tyres.wheels_pressure, b.tyres.wheels_pressure,
            "frame[{idx}].tyres.wheels_pressure"
        );
        assert_eq!(
            a.tyres.tyre_wear, b.tyres.tyre_wear,
            "frame[{idx}].tyres.tyre_wear"
        );
        assert_eq!(
            a.tyres.tyre_core_temperature, b.tyres.tyre_core_temperature,
            "frame[{idx}].tyres.tyre_core_temperature"
        );
        assert_eq!(
            a.tyres.brake_temp, b.tyres.brake_temp,
            "frame[{idx}].tyres.brake_temp"
        );
        assert_eq!(
            a.tyres.tyre_temp, b.tyres.tyre_temp,
            "frame[{idx}].tyres.tyre_temp"
        );

        // Powertrain (DriverInputs group)
        assert_eq!(
            a.powertrain.turbo_boost, b.powertrain.turbo_boost,
            "frame[{idx}].powertrain.turbo_boost"
        );
        assert_eq!(
            a.powertrain.kers_charge, b.powertrain.kers_charge,
            "frame[{idx}].powertrain.kers_charge"
        );
        assert_eq!(
            a.powertrain.drs, b.powertrain.drs,
            "frame[{idx}].powertrain.drs"
        );
        assert_eq!(
            a.powertrain.engine_brake, b.powertrain.engine_brake,
            "frame[{idx}].powertrain.engine_brake"
        );
        assert_eq!(
            a.powertrain.water_temp, b.powertrain.water_temp,
            "frame[{idx}].powertrain.water_temp"
        );

        // Session — timing-related fields go to Timing group, status fields to Environment
        assert_eq!(
            a.session.completed_laps, b.session.completed_laps,
            "frame[{idx}].session.completed_laps"
        );
        assert_eq!(
            a.session.position, b.session.position,
            "frame[{idx}].session.position"
        );
        assert_eq!(
            a.session.is_in_pit, b.session.is_in_pit,
            "frame[{idx}].session.is_in_pit"
        );
        assert_eq!(
            a.session.status, b.session.status,
            "frame[{idx}].session.status"
        );
        assert_eq!(
            a.session.session_time_left, b.session.session_time_left,
            "frame[{idx}].session.session_time_left"
        );

        // Timing (Timing group)
        assert_eq!(
            a.timing.i_current_time, b.timing.i_current_time,
            "frame[{idx}].timing.i_current_time"
        );
        assert_eq!(
            a.timing.i_best_time, b.timing.i_best_time,
            "frame[{idx}].timing.i_best_time"
        );
        assert_eq!(
            a.timing.i_last_time, b.timing.i_last_time,
            "frame[{idx}].timing.i_last_time"
        );
        assert_eq!(
            a.timing.i_delta_lap_time, b.timing.i_delta_lap_time,
            "frame[{idx}].timing.i_delta_lap_time"
        );
        assert_eq!(
            a.timing.i_estimated_lap_time, b.timing.i_estimated_lap_time,
            "frame[{idx}].timing.i_estimated_lap_time"
        );
        assert_eq!(
            a.timing.last_sector_time, b.timing.last_sector_time,
            "frame[{idx}].timing.last_sector_time"
        );

        // CarState (ColdStorage group)
        assert_eq!(
            a.car_state.cg_height, b.car_state.cg_height,
            "frame[{idx}].car_state.cg_height"
        );
        assert_eq!(
            a.car_state.brake_bias, b.car_state.brake_bias,
            "frame[{idx}].car_state.brake_bias"
        );
        assert_eq!(
            a.car_state.tc_level, b.car_state.tc_level,
            "frame[{idx}].car_state.tc_level"
        );
        assert_eq!(
            a.car_state.abs_level, b.car_state.abs_level,
            "frame[{idx}].car_state.abs_level"
        );
        assert_eq!(
            a.car_state.engine_map, b.car_state.engine_map,
            "frame[{idx}].car_state.engine_map"
        );

        // Environment (Environment group)
        assert_eq!(
            a.environment.air_density, b.environment.air_density,
            "frame[{idx}].environment.air_density"
        );
        assert_eq!(
            a.environment.air_temp, b.environment.air_temp,
            "frame[{idx}].environment.air_temp"
        );
        assert_eq!(
            a.environment.road_temp, b.environment.road_temp,
            "frame[{idx}].environment.road_temp"
        );
        assert_eq!(
            a.environment.wind_speed, b.environment.wind_speed,
            "frame[{idx}].environment.wind_speed"
        );
        assert_eq!(
            a.environment.surface_grip, b.environment.surface_grip,
            "frame[{idx}].environment.surface_grip"
        );

        // OtherCars (ColdStorage group)
        assert_eq!(
            a.other_cars.active_cars, b.other_cars.active_cars,
            "frame[{idx}].other_cars.active_cars"
        );
        assert_eq!(
            a.other_cars.player_car_id, b.other_cars.player_car_id,
            "frame[{idx}].other_cars.player_car_id"
        );
    }

    // =====================================================================
    // Test 1: Full roundtrip — write 3 laps × 5 frames, read all, verify
    // =====================================================================

    /// Write a 15-frame test session, read it back, and verify every
    /// frame field-by-field against the original.
    #[test]
    fn test_full_roundtrip() {
        let tmp = TempDir::new("v2_full_rt");
        let path = tmp.file_path("full_roundtrip.acctlm2");

        // Generate test data
        let (metadata, original_frames) = make_test_session(3, 5);
        assert_eq!(original_frames.len(), 15);

        // --- Write ---
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer = BinaryTelemetryWriterV2::create_file(&path, metadata.clone(), config)
            .expect("v2 writer: create_file");
        for frame in &original_frames {
            writer.write_frame(frame).expect("v2 writer: write_frame");
        }
        let _summary = writer.finish().expect("v2 writer: finish");

        // --- Read back ---
        let reader = BinaryTelemetryReaderV2::open(&path).expect("v2 reader: open");
        assert_eq!(reader.metadata().track_name, "test_track");
        assert_eq!(reader.metadata().car_model, "test_car");

        let read_frames = reader
            .read_all_frames()
            .expect("v2 reader: read_all_frames");
        assert_eq!(
            read_frames.len(),
            original_frames.len(),
            "frame count mismatch"
        );

        // --- Field-by-field comparison ---
        for (i, (orig, read)) in original_frames.iter().zip(read_frames.iter()).enumerate() {
            assert_frames_equal(orig, read, i);
        }
    }

    // =====================================================================
    // Test 2: Lap index roundtrip
    // =====================================================================

    /// Write a session, read the lap index back, and verify lap boundaries.
    #[test]
    fn test_lap_index_roundtrip() {
        let tmp = TempDir::new("v2_lap_idx");
        let path = tmp.file_path("lap_index.acctlm2");

        let fpl: u64 = 5;
        let (metadata, frames) = make_test_session(3, fpl);
        assert_eq!(frames.len(), 15);

        // --- Write ---
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer = BinaryTelemetryWriterV2::create_file(&path, metadata, config)
            .expect("v2 writer: create_file");
        for frame in &frames {
            writer.write_frame(frame).expect("v2 writer: write_frame");
        }
        writer.finish().expect("v2 writer: finish");

        // --- Read lap index ---
        let reader = BinaryTelemetryReaderV2::open(&path).expect("v2 reader: open");
        let lap_entries = reader.lap_index();
        assert_eq!(lap_entries.len(), 3, "expected 3 lap entries");

        // Lap 1: ticks 0..4
        assert_eq!(lap_entries[0].lap_number, 1);
        assert_eq!(lap_entries[0].start_tick, 0);
        assert_eq!(lap_entries[0].end_tick, fpl - 1);
        assert_eq!(lap_entries[0].sample_count, fpl as u32);

        // Lap 2: ticks 5..9
        assert_eq!(lap_entries[1].lap_number, 2);
        assert_eq!(lap_entries[1].start_tick, fpl);
        assert_eq!(lap_entries[1].end_tick, 2 * fpl - 1);
        assert_eq!(lap_entries[1].sample_count, fpl as u32);

        // Lap 3: ticks 10..14
        assert_eq!(lap_entries[2].lap_number, 3);
        assert_eq!(lap_entries[2].start_tick, 2 * fpl);
        assert_eq!(lap_entries[2].end_tick, 3 * fpl - 1);
        assert_eq!(lap_entries[2].sample_count, fpl as u32);
    }

    // =====================================================================
    // Test 3: Selective read — DriverInputs group only
    // =====================================================================

    /// Write a session, then read only the DriverInputs group and verify
    /// that all DriverInputs columns are present and no other groups leak in.
    #[test]
    fn test_selective_read_driver_inputs() {
        let tmp = TempDir::new("v2_sel_di");
        let path = tmp.file_path("selective_di.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        assert_eq!(frames.len(), 10);

        // --- Write ---
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer = BinaryTelemetryWriterV2::create_file(&path, metadata, config)
            .expect("v2 writer: create_file");
        for frame in &frames {
            writer.write_frame(frame).expect("v2 writer: write_frame");
        }
        writer.finish().expect("v2 writer: finish");

        // --- Read only DriverInputs ---
        let reader = BinaryTelemetryReaderV2::open(&path).expect("v2 reader: open");
        let result = reader
            .read_group_frames(&[GroupId::DriverInputs], None, None)
            .expect("v2 reader: read_group_frames");

        // Only one group should be present
        assert_eq!(result.len(), 1, "expected exactly 1 group in result");
        assert!(
            result.contains_key(&GroupId::DriverInputs),
            "DriverInputs group missing"
        );
        // Should NOT contain any other groups
        assert!(
            !result.contains_key(&GroupId::Timing),
            "Timing should not be present"
        );
        assert!(
            !result.contains_key(&GroupId::Tyres),
            "Tyres should not be present"
        );

        // DriverInputs has 30 columns (Controls 8 + Powertrain 22)
        let di_cols = result.get(&GroupId::DriverInputs).unwrap();
        assert_eq!(di_cols.len(), 30, "DriverInputs expects 30 columns");
        // Each column vector should have the right number of rows
        for (col_idx, col) in di_cols.iter().enumerate() {
            assert_eq!(
                col.len(),
                frames.len(),
                "DriverInputs column[{col_idx}] length mismatch"
            );
        }

        // Spot-check: first column (speed_kmh) should match frame[0] speed
        // Column order follows the schema, which for DriverInputs starts
        // with Controls fields then Powertrain fields.
        let speed_col = &di_cols[0];
        assert!(
            (speed_col[0] - 150.0).abs() < 1.0,
            "frame[0] speed_kmh ~150, got {}",
            speed_col[0]
        );
    }

    // =====================================================================
    // Test 4: Lap-bounded read
    // =====================================================================

    /// Write a 3-lap session, read only lap 2 frames, verify only that
    /// lap's frames are returned.
    #[test]
    fn test_lap_bounded_read() {
        let tmp = TempDir::new("v2_lap_bounded");
        let path = tmp.file_path("lap_bounded.acctlm2");

        let fpl: u64 = 5;
        let (metadata, frames) = make_test_session(3, fpl);
        assert_eq!(frames.len(), 15);

        // --- Write ---
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer = BinaryTelemetryWriterV2::create_file(&path, metadata, config)
            .expect("v2 writer: create_file");
        for frame in &frames {
            writer.write_frame(frame).expect("v2 writer: write_frame");
        }
        writer.finish().expect("v2 writer: finish");

        // --- Read lap 2 only (1-based) ---
        let reader = BinaryTelemetryReaderV2::open(&path).expect("v2 reader: open");
        let lap2_frames = reader
            .read_lap_frames(2)
            .expect("v2 reader: read_lap_frames");

        assert_eq!(
            lap2_frames.len(),
            fpl as usize,
            "lap 2 should have exactly {fpl} frames"
        );

        // Verify every returned frame belongs to lap 2
        for (i, frame) in lap2_frames.iter().enumerate() {
            // Lap 2 covers ticks [fpl, 2*fpl) → completed_laps = 1
            assert_eq!(
                frame.session.completed_laps, 1,
                "lap 2 frame[{i}]: expected completed_laps=1, got {}",
                frame.session.completed_laps
            );
            // The expected tick range for lap 2
            let expected_tick = fpl + i as u64;
            assert_eq!(
                frame.sample_tick, expected_tick,
                "lap 2 frame[{i}]: expected tick={expected_tick}, got {}",
                frame.sample_tick
            );
        }

        // Verify the frames match the original
        for (i, (orig, read)) in frames[fpl as usize..2 * fpl as usize]
            .iter()
            .zip(lap2_frames.iter())
            .enumerate()
        {
            assert_frames_equal(orig, read, fpl as usize + i);
        }
    }

    // =====================================================================
    // Test 5: Cross-group selective read
    // =====================================================================

    /// Write a session, read DriverInputs + Timing groups, verify that
    /// Tyres columns are NOT present.
    #[test]
    fn test_cross_group_selective() {
        let tmp = TempDir::new("v2_cross_group");
        let path = tmp.file_path("cross_group.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        assert_eq!(frames.len(), 10);

        // --- Write ---
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer = BinaryTelemetryWriterV2::create_file(&path, metadata, config)
            .expect("v2 writer: create_file");
        for frame in &frames {
            writer.write_frame(frame).expect("v2 writer: write_frame");
        }
        writer.finish().expect("v2 writer: finish");

        // --- Read DriverInputs + Timing ---
        let reader = BinaryTelemetryReaderV2::open(&path).expect("v2 reader: open");
        let result = reader
            .read_group_frames(&[GroupId::DriverInputs, GroupId::Timing], None, None)
            .expect("v2 reader: read_group_frames");

        // Exactly 2 groups present
        assert_eq!(
            result.len(),
            2,
            "expected exactly 2 groups (DriverInputs + Timing), got {}",
            result.len()
        );

        // Both requested groups present
        assert!(
            result.contains_key(&GroupId::DriverInputs),
            "DriverInputs should be present"
        );
        assert!(
            result.contains_key(&GroupId::Timing),
            "Timing should be present"
        );

        // Tyres group must NOT be present
        assert!(
            !result.contains_key(&GroupId::Tyres),
            "Tyres should NOT be present in selective read"
        );
        // VehicleDynamics should not be present either
        assert!(
            !result.contains_key(&GroupId::VehicleDynamics),
            "VehicleDynamics should NOT be present"
        );
        // ColdStorage should not be present
        assert!(
            !result.contains_key(&GroupId::ColdStorage),
            "ColdStorage should NOT be present"
        );

        // Verify column counts
        // DriverInputs = 30 columns (Controls 8 + Powertrain 22)
        let di_cols = result.get(&GroupId::DriverInputs).unwrap();
        assert_eq!(di_cols.len(), 30, "DriverInputs: expected 30 columns");
        // Timing = 26 columns
        let timing_cols = result.get(&GroupId::Timing).unwrap();
        assert_eq!(timing_cols.len(), 26, "Timing: expected 26 columns");

        // Each column has the right number of rows
        // (TYPE_BYTES columns have sub_count × row_count values, so we
        //  check that the length is divisible by 10 with at least 10 values)
        for (col_idx, col) in di_cols.iter().enumerate() {
            assert!(
                col.len() >= 10 && col.len() % 10 == 0,
                "DriverInputs column[{col_idx}] length {} should be ≥10 and divisible by 10",
                col.len()
            );
        }
        for (col_idx, col) in timing_cols.iter().enumerate() {
            assert!(
                col.len() >= 10 && col.len() % 10 == 0,
                "Timing column[{col_idx}] length {} should be ≥10 and divisible by 10",
                col.len()
            );
        }
    }

    // =====================================================================
    // Test 6: Typed read_all_*_v2 methods — individual substructures
    // =====================================================================

    /// Verify `read_all_controls_v2()` returns correct control data.
    #[test]
    fn test_read_all_controls_v2() {
        let tmp = TempDir::new("v2_typed_ctrl");
        let path = tmp.file_path("ctrl.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader.read_all_controls_v2().expect("read_all_controls_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(s.sample_tick, f.sample_tick, "frame[{i}].sample_tick");
            assert_eq!(s.timestamp_ns, f.timestamp_ns, "frame[{i}].timestamp_ns");
            assert_eq!(s.speed_kmh, f.controls.speed_kmh, "frame[{i}].speed_kmh");
            assert_eq!(s.gas, f.controls.gas, "frame[{i}].gas");
            assert_eq!(s.brake, f.controls.brake, "frame[{i}].brake");
            assert_eq!(s.clutch, f.controls.clutch, "frame[{i}].clutch");
            assert_eq!(
                s.steer_angle, f.controls.steer_angle,
                "frame[{i}].steer_angle"
            );
            assert_eq!(s.gear, f.controls.gear, "frame[{i}].gear");
            assert_eq!(s.rpms, f.controls.rpms, "frame[{i}].rpms");
            assert_eq!(s.fuel, f.controls.fuel, "frame[{i}].fuel");
        }
    }

    /// Verify `read_all_motion_v2()` returns correct motion data.
    #[test]
    fn test_read_all_motion_v2() {
        let tmp = TempDir::new("v2_typed_motion");
        let path = tmp.file_path("motion.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader.read_all_motion_v2().expect("read_all_motion_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(s.sample_tick, f.sample_tick, "frame[{i}].sample_tick");
            assert_eq!(s.velocity, f.motion.velocity, "frame[{i}].velocity");
            assert_eq!(s.acc_g, f.motion.acc_g, "frame[{i}].acc_g");
            assert_eq!(s.heading, f.motion.heading, "frame[{i}].heading");
            assert_eq!(s.pitch, f.motion.pitch, "frame[{i}].pitch");
            assert_eq!(s.roll, f.motion.roll, "frame[{i}].roll");
        }
    }

    /// Verify `read_all_tyres_v2()` returns correct tyre data.
    #[test]
    fn test_read_all_tyres_v2() {
        let tmp = TempDir::new("v2_typed_tyres");
        let path = tmp.file_path("tyres.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader.read_all_tyres_v2().expect("read_all_tyres_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(s.wheel_slip, f.tyres.wheel_slip, "frame[{i}].wheel_slip");
            assert_eq!(s.wheel_load, f.tyres.wheel_load, "frame[{i}].wheel_load");
            assert_eq!(
                s.wheels_pressure, f.tyres.wheels_pressure,
                "frame[{i}].wheels_pressure"
            );
            assert_eq!(s.tyre_wear, f.tyres.tyre_wear, "frame[{i}].tyre_wear");
            assert_eq!(s.tyre_temp, f.tyres.tyre_temp, "frame[{i}].tyre_temp");
            assert_eq!(s.brake_temp, f.tyres.brake_temp, "frame[{i}].brake_temp");
        }
    }

    /// Verify `read_all_powertrain_v2()` returns correct powertrain data.
    #[test]
    fn test_read_all_powertrain_v2() {
        let tmp = TempDir::new("v2_typed_ptrain");
        let path = tmp.file_path("ptrain.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader
            .read_all_powertrain_v2()
            .expect("read_all_powertrain_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(
                s.turbo_boost, f.powertrain.turbo_boost,
                "frame[{i}].turbo_boost"
            );
            assert_eq!(
                s.kers_charge, f.powertrain.kers_charge,
                "frame[{i}].kers_charge"
            );
            assert_eq!(s.drs, f.powertrain.drs, "frame[{i}].drs");
            assert_eq!(
                s.engine_brake, f.powertrain.engine_brake,
                "frame[{i}].engine_brake"
            );
            assert_eq!(
                s.water_temp, f.powertrain.water_temp,
                "frame[{i}].water_temp"
            );
        }
    }

    /// Verify `read_all_session_v2()` returns correct session data.
    #[test]
    fn test_read_all_session_v2() {
        let tmp = TempDir::new("v2_typed_sess");
        let path = tmp.file_path("sess.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader.read_all_session_v2().expect("read_all_session_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(
                s.completed_laps, f.session.completed_laps,
                "frame[{i}].completed_laps"
            );
            assert_eq!(s.position, f.session.position, "frame[{i}].position");
            assert_eq!(s.is_in_pit, f.session.is_in_pit, "frame[{i}].is_in_pit");
            assert_eq!(s.status, f.session.status, "frame[{i}].status");
            assert_eq!(s.session, f.session.session, "frame[{i}].session");
            assert_eq!(
                s.global_yellow, f.session.global_yellow,
                "frame[{i}].global_yellow"
            );
        }
    }

    /// Verify `read_all_timing_v2()` returns correct timing data.
    #[test]
    fn test_read_all_timing_v2() {
        let tmp = TempDir::new("v2_typed_timing");
        let path = tmp.file_path("timing.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader.read_all_timing_v2().expect("read_all_timing_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(
                s.i_current_time, f.timing.i_current_time,
                "frame[{i}].i_current_time"
            );
            assert_eq!(
                s.i_last_time, f.timing.i_last_time,
                "frame[{i}].i_last_time"
            );
            assert_eq!(
                s.i_best_time, f.timing.i_best_time,
                "frame[{i}].i_best_time"
            );
            assert_eq!(s.i_split, f.timing.i_split, "frame[{i}].i_split");
            assert_eq!(
                s.fuel_estimated_laps, f.timing.fuel_estimated_laps,
                "frame[{i}].fuel_estimated_laps"
            );
        }
    }

    /// Verify `read_all_car_state_v2()` returns correct car state data.
    #[test]
    fn test_read_all_car_state_v2() {
        let tmp = TempDir::new("v2_typed_carstate");
        let path = tmp.file_path("carstate.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader
            .read_all_car_state_v2()
            .expect("read_all_car_state_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(
                s.car_damage, f.car_state.car_damage,
                "frame[{i}].car_damage"
            );
            assert_eq!(s.cg_height, f.car_state.cg_height, "frame[{i}].cg_height");
            assert_eq!(
                s.brake_bias, f.car_state.brake_bias,
                "frame[{i}].brake_bias"
            );
            assert_eq!(
                s.engine_map, f.car_state.engine_map,
                "frame[{i}].engine_map"
            );
        }
    }

    /// Verify `read_all_environment_v2()` returns correct environment data.
    #[test]
    fn test_read_all_environment_v2() {
        let tmp = TempDir::new("v2_typed_env");
        let path = tmp.file_path("env.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader
            .read_all_environment_v2()
            .expect("read_all_environment_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(s.air_temp, f.environment.air_temp, "frame[{i}].air_temp");
            assert_eq!(s.road_temp, f.environment.road_temp, "frame[{i}].road_temp");
            assert_eq!(
                s.wind_speed, f.environment.wind_speed,
                "frame[{i}].wind_speed"
            );
            assert_eq!(
                s.rain_intensity, f.environment.rain_intensity,
                "frame[{i}].rain_intensity"
            );
        }
    }

    /// Verify `read_all_other_cars_v2()` returns correct other cars data.
    #[test]
    fn test_read_all_other_cars_v2() {
        let tmp = TempDir::new("v2_typed_ocars");
        let path = tmp.file_path("ocars.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");
        let samples = reader
            .read_all_other_cars_v2()
            .expect("read_all_other_cars_v2");

        assert_eq!(samples.len(), frames.len());

        for (i, (s, f)) in samples.iter().zip(frames.iter()).enumerate() {
            assert_eq!(
                s.active_cars, f.other_cars.active_cars,
                "frame[{i}].active_cars"
            );
            assert_eq!(
                s.player_car_id, f.other_cars.player_car_id,
                "frame[{i}].player_car_id"
            );
        }
    }

    // =====================================================================
    // Test 7: ItemKey-based reader
    // =====================================================================

    /// Verify `read_item_frames` resolves `raw:controls.speed_kmh`.
    #[test]
    fn test_read_item_frames_controls_speed() {
        use module_live_telemetry::item_key::ItemKey;

        let tmp = TempDir::new("v2_itemkey_ctrl");
        let path = tmp.file_path("itemkey_ctrl.acctlm2");

        let (metadata, frames) = make_test_session(2, 5);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");

        let key = ItemKey::parse("raw:controls.speed_kmh").expect("parse key");
        // Read frames 0..4 (5 frames)
        let values = reader
            .read_item_frames(&key, 0, 4)
            .expect("read_item_frames");

        assert_eq!(values.len(), 5, "should return 5 values for ticks 0..4");

        for (i, v) in values.iter().enumerate() {
            let expected = frames[i].controls.speed_kmh as f64;
            assert!(
                (v - expected).abs() < 0.001,
                "frame[{i}]: expected speed_kmh={expected}, got {v}"
            );
        }
    }

    /// Verify `read_item_frames` with `raw:motion.heading`.
    #[test]
    fn test_read_item_frames_motion_heading() {
        use module_live_telemetry::item_key::ItemKey;

        let tmp = TempDir::new("v2_itemkey_mot");
        let path = tmp.file_path("itemkey_mot.acctlm2");

        let (metadata, frames) = make_test_session(1, 3);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");

        let key = ItemKey::parse("raw:motion.heading").expect("parse key");
        let values = reader
            .read_item_frames(&key, 0, 2)
            .expect("read_item_frames");

        assert_eq!(values.len(), 3, "should return 3 values");

        for (i, v) in values.iter().enumerate() {
            let expected = frames[i].motion.heading as f64;
            assert!(
                (v - expected).abs() < 0.001,
                "frame[{i}]: expected heading={expected}, got {v}"
            );
        }
    }

    /// Verify `read_item_frames` rejects `calc:*` items.
    #[test]
    fn test_read_item_frames_calc_rejected() {
        use module_live_telemetry::item_key::ItemKey;

        let tmp = TempDir::new("v2_itemkey_calc");
        let path = tmp.file_path("itemkey_calc.acctlm2");

        let (metadata, frames) = make_test_session(1, 2);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");

        let key = ItemKey::parse("calc:delta_best").expect("parse key");
        let result = reader.read_item_frames(&key, 0, 1);
        assert!(result.is_err(), "calc items should be rejected");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("calc"), "error should mention calc: {err}");
    }

    /// Verify `read_item_frames` with `raw:tyres.brake_temp` (TYPE_BYTES column).
    #[test]
    fn test_read_item_frames_tyres_bytes_column() {
        use module_live_telemetry::item_key::ItemKey;

        let tmp = TempDir::new("v2_itemkey_tyre");
        let path = tmp.file_path("itemkey_tyre.acctlm2");

        let (metadata, frames) = make_test_session(1, 2);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata, config).expect("create_file");
        for f in &frames {
            writer.write_frame(f).expect("write_frame");
        }
        writer.finish().expect("finish");

        let reader = BinaryTelemetryReaderV2::open(&path).expect("open");

        let key = ItemKey::parse("raw:tyres.brake_temp").expect("parse key");
        // read all 2 frames, brake_temp has 4 sub-values per frame → 8 values
        let values = reader
            .read_item_frames(&key, 0, 1)
            .expect("read_item_frames");

        assert_eq!(
            values.len(),
            8,
            "brake_temp: 2 frames × 4 wheels = 8 values"
        );

        // Verify first frame's brake_temp = arr4(200.0 + ...)
        for wheel in 0..4 {
            let expected = frames[0].tyres.brake_temp[wheel] as f64;
            let got = values[wheel];
            assert!(
                (got - expected).abs() < 0.01,
                "wheel[{wheel}]: expected {expected}, got {got}"
            );
        }
    }
}
