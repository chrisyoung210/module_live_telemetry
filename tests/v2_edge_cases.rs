//! V2 Format: Edge case test suite.
//!
//! Covers empty sessions, CRC32 corruption, truncation, mmap lifecycle,
//! schema mismatch, mid-row-group lap boundaries, BYTES column preservation,
//! and single-frame sessions.
//!
//! All tests (except `test_test_infrastructure`) are marked
//! `#[ignore = "requires v2 reader/writer"]` and represent the expected
//! API contract for the yet-to-be-implemented v2 reader/writer.
//!
//! # Expected API
//!
//! | Type | Method | Description |
//! |------|--------|-------------|
//! | `BinaryTelemetryWriterV2` | `create(Write, SessionMetadata)` | Create writer |
//! | `BinaryTelemetryWriterV2` | `write_frame(&TelemetryFrame)` | Write a frame |
//! | `BinaryTelemetryWriterV2` | `finish()` → `Vec<u8>` | Finalize, return bytes |
//! | `BinaryTelemetryReaderV2` | `from_bytes(&[u8])` | Open from buffer |
//! | `BinaryTelemetryReaderV2` | `read_all_frames()` → `Vec<TelemetryFrame>` | Read all frames |
//! | `BinaryTelemetryReaderV2` | `metadata()` → `&SessionMetadata` | Session metadata |
//! | `BinaryTelemetryReaderV2` | `read_lap(u32)` → `Vec<TelemetryFrame>` | Read frames for a lap |
//! | `MmapFile` | `open(path)` | Open file via mmap |
//! | `MmapFile` | `as_slice()` → `&[u8]` | View as byte slice |
//! | `MmapFile` | `close()` | Release mmap (Windows) |

use module_live_telemetry::format_v2::{
    ColumnId, FileHeaderV2, GroupId, HEADER_V2_SIZE, TYPE_BYTES, TYPE_F32,
};
use module_live_telemetry::TelemetryFrame;
use std::io::Cursor;

mod test_data;

// ---------------------------------------------------------------------------
// Helper: run_roundtrip
// ---------------------------------------------------------------------------

/// Helper: roundtrip `frames` through expected v2 writer/reader.
///
/// This helper is used by all ignored tests to avoid duplicating the
/// writer/reader API call pattern.  When the writer/reader are implemented,
/// a single change here updates all tests.
#[allow(unused)]
fn run_roundtrip(
    metadata: &module_live_telemetry::SessionMetadata,
    frames: &[TelemetryFrame],
) -> Vec<TelemetryFrame> {
    // Expected API (to be implemented by Tasks 6-9):
    //
    //   let mut buf = Vec::new();
    //   let mut writer = BinaryTelemetryWriterV2::create(&mut buf, metadata.clone()).unwrap();
    //   for frame in frames {
    //       writer.write_frame(frame).unwrap();
    //   }
    //   writer.finish().unwrap();
    //   let reader = BinaryTelemetryReaderV2::from_bytes(&buf).unwrap();
    //   reader.read_all_frames().unwrap()
    //
    // For now, placeholder to suppress unused warnings.
    let _ = metadata;
    let _ = frames;
    todo!("BinaryTelemetryWriterV2 / BinaryTelemetryReaderV2 not yet implemented")
}

// ---------------------------------------------------------------------------
// Self-verifying: test_test_infrastructure (passes immediately)
// ---------------------------------------------------------------------------

/// Verify that the test infrastructure (test data generators, format_v2 types)
/// is functional.  This test MUST pass immediately without any v2 writer/reader.
#[test]
fn test_test_infrastructure() {
    // -- test_data generators exist and produce deterministic output --
    let empty = test_data::make_test_session_empty();
    assert!(empty.1.is_empty());

    let single = test_data::make_test_session_single_frame();
    assert_eq!(single.1.len(), 1);

    let (meta, frames) = test_data::make_test_session(3, 5);
    assert_eq!(meta.track_name, "test_track");
    assert_eq!(frames.len(), 15);

    // -- Every frame has distinctive values --
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.sample_tick, i as u64, "frame[{i}].sample_tick");
        assert_eq!(
            f.controls.speed_kmh,
            150.0 + i as f32 * 0.1,
            "frame[{i}].speed_kmh"
        );
    }

    // -- format_v2 types and constants are importable --
    // MAGIC_ACT2 is private; we still verify the constant length/size works
    assert_eq!(HEADER_V2_SIZE, 64);
    assert_eq!(TYPE_BYTES, 0x05);
    assert_eq!(TYPE_F32, 0x03);
    assert_eq!(ColumnId::SampleTick as u16, 1);
    assert_eq!(GroupId::FrameMeta as usize, 0);

    // -- FileHeaderV2 roundtrips in-process --
    let hdr = FileHeaderV2 {
        schema_offset: 100,
        metadata_offset: 200,
        first_row_group_offset: 300,
        footer_offset: 400,
        created_unix_ns: 1_700_000_000_000_000_000,
        poll_hz: 120_000,
    };
    let mut buf = Vec::new();
    hdr.write_to(&mut buf).unwrap();
    assert_eq!(buf.len(), HEADER_V2_SIZE);
    let mut cursor = Cursor::new(&buf);
    let got = FileHeaderV2::read_from(&mut cursor).unwrap();
    assert_eq!(hdr, got);
}

// ---------------------------------------------------------------------------
// Empty session
// ---------------------------------------------------------------------------

/// Write a session with zero frames, read back — must not panic and must
/// return an empty frame vector.
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_empty_session() {
    let (_meta, frames) = test_data::make_test_session_empty();
    let result = run_roundtrip(&_meta, &frames);
    assert!(result.is_empty(), "expected empty frame list");
}

// ---------------------------------------------------------------------------
// Single-frame session
// ---------------------------------------------------------------------------

/// Write exactly one frame, read back, verify all fields match.
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_single_frame_session() {
    let (_meta, frames) = test_data::make_test_session_single_frame();
    let result = run_roundtrip(&_meta, &frames);
    assert_eq!(result.len(), 1, "expected exactly one frame");
    let original = &frames[0];
    let roundtripped = &result[0];
    assert_eq!(original.sample_tick, roundtripped.sample_tick);
    assert_eq!(original.timestamp_ns, roundtripped.timestamp_ns);
    assert_eq!(original.controls.speed_kmh, roundtripped.controls.speed_kmh);
    assert_eq!(original.motion.velocity, roundtripped.motion.velocity);
    assert_eq!(original.tyres.tyre_temp, roundtripped.tyres.tyre_temp);
    assert_eq!(
        original.timing.i_current_time,
        roundtripped.timing.i_current_time
    );
    assert_eq!(
        original.car_state.car_damage,
        roundtripped.car_state.car_damage
    );
}

// ---------------------------------------------------------------------------
// Partial row group (non-divisible frame count)
// ---------------------------------------------------------------------------

/// Write N frames where N is not divisible by chunk_rows.
/// Last row group must have fewer frames than full groups.
/// All frames must roundtrip correctly.
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_partial_row_group() {
    // 300 frames with chunk_rows=256 → row groups: [0..256, 256..300]
    // The last group has 44 frames.
    let chunk_rows = 256;
    let total = 300;
    let metadata = test_data::make_test_metadata("spa", "porsche_992_gt3_r");
    // Override chunk_rows to 256 for this test
    let mut meta = metadata;
    meta.chunk_rows = chunk_rows;
    let frames: Vec<TelemetryFrame> = (0..total)
        .map(|t| test_data::make_test_frame(t as u64, 1))
        .collect();

    let result = run_roundtrip(&meta, &frames);
    assert_eq!(result.len(), total, "all frames must roundtrip");

    // Verify all frame data
    for (i, (original, roundtripped)) in frames.iter().zip(result.iter()).enumerate() {
        assert_eq!(original.sample_tick, roundtripped.sample_tick, "i={i}");
        assert_eq!(
            original.controls.speed_kmh, roundtripped.controls.speed_kmh,
            "i={i}"
        );
    }
}

// ---------------------------------------------------------------------------
// Mid-row-group lap boundary
// ---------------------------------------------------------------------------

/// Write a session where a lap boundary falls within a row group.
/// Example: 5 frames/lap with chunk_rows=10 → row group 0 covers
/// frames 0-9 (laps 1 and 2).  Lap 2 starts mid-group.
/// Lap-bounded reads must return correct frames.
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_mid_group_lap_boundary() {
    let frames_per_lap = 5;
    let chunk_rows = 10; // each row group holds exactly 2 laps
    let lap_count = 4; // frames 0..=19, laps 1..=4

    let (meta, frames) = test_data::make_test_session(lap_count, frames_per_lap);
    let mut meta = meta;
    meta.chunk_rows = chunk_rows;

    let result = run_roundtrip(&meta, &frames);
    assert_eq!(result.len(), frames.len(), "all frames must roundtrip");

    // Verify lap 2 starts mid-group (frame 5 within the first row group 0..=9)
    // Lap 2 frames = frames[5..=9]
    for lap_num in 1..=lap_count {
        let lap_start = ((lap_num - 1) as usize) * (frames_per_lap as usize);
        let lap_end = (lap_start + frames_per_lap as usize).min(result.len());

        let original_lap: Vec<&TelemetryFrame> = frames[lap_start..lap_end].iter().collect();
        let roundtripped_lap: Vec<&TelemetryFrame> = result[lap_start..lap_end].iter().collect();

        assert_eq!(
            original_lap.len(),
            roundtripped_lap.len(),
            "lap {lap_num} length mismatch"
        );
        for (orig, rt) in original_lap.iter().zip(roundtripped_lap.iter()) {
            assert_eq!(
                orig.sample_tick, rt.sample_tick,
                "lap {lap_num} sample_tick"
            );
            assert_eq!(
                orig.controls.speed_kmh, rt.controls.speed_kmh,
                "lap {lap_num} speed"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// BYTES column preservation
// ---------------------------------------------------------------------------

/// Verify that TYPE_BYTES columns (velocity, wheel_slip, tyre_temp)
/// preserve all sub-values after roundtrip.
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_bytes_columns_preserved() {
    let (_meta, frames) = test_data::make_test_session_single_frame();
    let original = &frames[0];

    // Motion.velocity is a [f32; 3] — stored as TYPE_BYTES blob
    let expected_velocity = original.motion.velocity;
    assert_eq!(
        expected_velocity.len(),
        3,
        "velocity must have 3 sub-values"
    );

    // TyreSample.wheel_slip is a [f32; 4] — stored as TYPE_BYTES blob
    let expected_wheel_slip = original.tyres.wheel_slip;
    assert_eq!(
        expected_wheel_slip.len(),
        4,
        "wheel_slip must have 4 sub-values"
    );

    // TyreSample.tyre_temp is a [f32; 4] — stored as TYPE_BYTES blob
    let expected_tyre_temp = original.tyres.tyre_temp;
    assert_eq!(
        expected_tyre_temp.len(),
        4,
        "tyre_temp must have 4 sub-values"
    );

    let result = run_roundtrip(&_meta, &frames);
    assert_eq!(result.len(), 1, "expected one frame");
    let rt = &result[0];

    // Verify each sub-value is preserved exactly
    for (i, (&orig, &rt_val)) in expected_velocity
        .iter()
        .zip(rt.motion.velocity.iter())
        .enumerate()
    {
        assert!(
            (orig - rt_val).abs() < f32::EPSILON,
            "velocity[{i}]: expected {orig}, got {rt_val}"
        );
    }
    for (i, (&orig, &rt_val)) in expected_wheel_slip
        .iter()
        .zip(rt.tyres.wheel_slip.iter())
        .enumerate()
    {
        assert!(
            (orig - rt_val).abs() < f32::EPSILON,
            "wheel_slip[{i}]: expected {orig}, got {rt_val}"
        );
    }
    for (i, (&orig, &rt_val)) in expected_tyre_temp
        .iter()
        .zip(rt.tyres.tyre_temp.iter())
        .enumerate()
    {
        assert!(
            (orig - rt_val).abs() < f32::EPSILON,
            "tyre_temp[{i}]: expected {orig}, got {rt_val}"
        );
    }

    // Verify all 12 BYTES sub-values roundtrip via the full frame comparison
    assert_eq!(
        original.motion.velocity, rt.motion.velocity,
        "velocity array mismatch"
    );
    assert_eq!(
        original.tyres.wheel_slip, rt.tyres.wheel_slip,
        "wheel_slip array mismatch"
    );
    assert_eq!(
        original.tyres.tyre_temp, rt.tyres.tyre_temp,
        "tyre_temp array mismatch"
    );
}

// ---------------------------------------------------------------------------
// Schema mismatch
// ---------------------------------------------------------------------------

/// Attempt to read a file whose schema hash does not match the reader's
/// expected schema.  The reader must return an error (not panic).
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_schema_mismatch() {
    // Write a file with schema A
    let (_meta, frames) = test_data::make_test_session(1, 10);
    let bytes_a = run_roundtrip(&_meta, &frames);
    // bytes_a now holds the buffer; but we need it as raw bytes to manipulate.
    // To test schema mismatch, we need to write raw bytes where the schema
    // block has a different set of columns / hash than the reader expects.
    //
    // Expected behaviour:
    //   let reader = BinaryTelemetryReaderV2::from_bytes(&bytes_a);
    //   // If the file has schema A, the reader validates it against its
    //   // baked-in schema hash and returns Err if they differ.
    //   assert!(reader.is_err());
    let _ = bytes_a;
    let _ = frames;
    todo!("BinaryTelemetryReaderV2 schema mismatch test — implement when reader exists")
}

// ---------------------------------------------------------------------------
// CRC32 corruption
// ---------------------------------------------------------------------------

/// Write a valid file, manually corrupt a column entry's CRC32 bytes,
/// then attempt to read.  The decoder must return an error.
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_crc32_corruption() {
    // Step 1: write a valid file with known frames.
    let (_meta, _frames) = test_data::make_test_session(1, 5);
    let original_bytes: Vec<u8> = {
        // Produce the raw bytes via the v2 writer
        let buf = Vec::new();
        // BinaryTelemetryWriterV2::create(&mut buf, ...) -> write frames -> finish
        // For now, use a placeholder buffer
        buf
    };

    // Step 2: locate the CRC32 field for a known column entry and corrupt it.
    // ColumnEntryV2 layout (40 bytes):
    //   column_id: u16 (2), codec+value_type: u8+u8 (2), byte_len: u32 (4),
    //   crc32: u32 (4), ... rest
    // CRC32 is at offset 8 within each ColumnEntryV2 (after column_id, codec, value_type, byte_len).
    //
    // To find it: scan the file between schema block and first row group
    // for the specific column_id byte pattern, then flip a bit in crc32.
    let mut corrupted = original_bytes.clone();
    if corrupted.len() > HEADER_V2_SIZE + 100 {
        // Introduce corruption at an arbitrary offset past the header
        // (real test will use the exact CRC32 position from schema parsing)
        corrupted[HEADER_V2_SIZE + 80] ^= 0xFF;
    }

    // Step 3: attempt to read the corrupted file.
    // Expected:
    //   let reader = BinaryTelemetryReaderV2::from_bytes(&corrupted);
    //   assert!(reader.is_err(), "CRC32 corruption should produce error");

    let _ = corrupted;
    todo!("CRC32 corruption test — implement when reader exists")
}

// ---------------------------------------------------------------------------
// Truncated file
// ---------------------------------------------------------------------------

/// Write a valid file, truncate the last 100 bytes, then attempt an mmap
/// read.  The reader must return an error (not panic / segfault).
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_truncated_file() {
    // Step 1: write a small session to a temp file.
    let dir = std::env::temp_dir();
    let path = dir.join("acctlm2_trunc_test.tmp");

    let (_meta, frames) = test_data::make_test_session(1, 10);
    {
        // Write via BinaryTelemetryWriterV2
        //   let mut f = std::fs::File::create(&path).unwrap();
        //   let mut writer = BinaryTelemetryWriterV2::create(&mut f, _meta.clone()).unwrap();
        //   for frame in &frames { writer.write_frame(frame).unwrap(); }
        //   writer.finish().unwrap();
    }

    // Step 2: truncate the last 100 bytes.
    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if len > 100 {
        let f = std::fs::File::open(&path).unwrap();
        f.set_len(len - 100).ok();
    }

    // Step 3: attempt to read the truncated file.
    // Expected:
    //   let reader = BinaryTelemetryReaderV2::from_bytes(&buf);
    //   assert!(reader.is_err(), "truncated file should produce error");

    // Clean up
    let _ = std::fs::remove_file(&path);
    let _ = frames;
    todo!("truncated file test — implement when reader exists")
}

// ---------------------------------------------------------------------------
// mmap lifecycle (Windows sharing violation check)
// ---------------------------------------------------------------------------

/// Open a file with mmap, read data, close the mmap, then verify the file
/// can be renamed or deleted without a Windows ERROR_SHARING_VIOLATION.
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_mmap_lifecycle() {
    let dir = std::env::temp_dir();
    let original_path = dir.join("acctlm2_mmap_lifecycle.tmp");
    let renamed_path = dir.join("acctlm2_mmap_lifecycle_renamed.tmp");

    // Step 1: write a session to temp file.
    let (_meta, frames) = test_data::make_test_session(2, 10);
    {
        // Write via BinaryTelemetryWriterV2
        //   let mut f = std::fs::File::create(&original_path).unwrap();
        //   let mut writer = BinaryTelemetryWriterV2::create(&mut f, _meta.clone()).unwrap();
        //   for frame in &frames { writer.write_frame(frame).unwrap(); }
        //   writer.finish().unwrap();
    }

    // Step 2: mmap the file, read some data.
    // Expected API:
    //   let mmap = MmapFile::open(&original_path).unwrap();
    //   let slice = mmap.as_slice();
    //   assert!(slice.len() > HEADER_V2_SIZE);
    //   let _magic = &slice[0..4];
    //   drop(mmap);  // or mmap.close();

    // Step 3: verify file can be renamed (no sharing violation).
    //   std::fs::rename(&original_path, &renamed_path)
    //       .expect("file must be renameable after mmap is dropped");
    //   std::fs::remove_file(&renamed_path).ok();

    // Clean up (in case rename failed)
    let _ = std::fs::remove_file(&original_path);
    let _ = std::fs::remove_file(&renamed_path);
    let _ = frames;
    todo!("mmap lifecycle test — implement when reader exists")
}

// ---------------------------------------------------------------------------
// File header validation (bad magic)
// ---------------------------------------------------------------------------

/// Attempt to read a file with wrong magic bytes (not b"ACT2").
/// The reader must return an error immediately without panicking.
#[test]
#[ignore = "requires v2 reader/writer"]
fn test_file_header_validation() {
    // Create a buffer with wrong magic bytes
    let bad_bytes: Vec<u8> = {
        let mut b = vec![0u8; HEADER_V2_SIZE];
        // Write a wrong magic
        b[0..4].copy_from_slice(b"BAD!");
        b
    };

    // Expected:
    //   let reader = BinaryTelemetryReaderV2::from_bytes(&bad_bytes);
    //   assert!(reader.is_err(), "wrong magic must produce error");
    //
    // Also verify FileHeaderV2::read_from returns error for bad magic:
    let mut cursor = Cursor::new(&bad_bytes);
    let result = FileHeaderV2::read_from(&mut cursor);
    assert!(result.is_err(), "FileHeaderV2 must reject bad magic");
}
