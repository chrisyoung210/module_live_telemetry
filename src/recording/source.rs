//! Telemetry source abstraction
//!
//! Defines `TelemetrySource` trait for injectable telemetry data.
//! Includes a real ACC shared-memory adapter and a scripted fake
//! for testing without real ACC shared memory.

use crate::error::{TelemetryError, TelemetryResult};
use crate::reader::BinaryTelemetryReader;
use crate::shmem::{AccGameStatus, AccSessionInfo, AccSharedMemoryReader};
use crate::types::SessionMetadata;
use crate::writer::TelemetryFrame;
use std::collections::VecDeque;

/// Abstract telemetry data source.
///
/// Implementations provide status, session info, static bytes,
/// and telemetry frames. The `Send` bound allows the source to
/// be moved into a recording thread.
pub trait TelemetrySource: Send {
    /// Current ACC game status (Live, Off, Pause, etc.)
    fn status(&mut self) -> TelemetryResult<AccGameStatus>;

    /// Session metadata (track name, car model)
    fn session_info(&mut self) -> TelemetryResult<AccSessionInfo>;

    /// Raw static page bytes (for metadata)
    fn read_static_bytes(&mut self) -> TelemetryResult<Vec<u8>>;

    /// Read next telemetry frame (with packet-ID deduplication).
    ///
    /// Returns `Ok(None)` when no new frame is available (e.g. packet
    /// ID unchanged or ACC not live). Returns `Err` on shared-memory
    /// disconnect or other fatal error.
    ///
    /// Used by real-time display (e.g. `serve` command) where duplicate
    /// frames are undesirable.
    fn read_telemetry_frame(
        &mut self,
        sample_tick: u64,
        timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>>;

    /// Read every polled frame (no packet-ID deduplication).
    ///
    /// Unlike `read_telemetry_frame`, this method always produces a frame
    /// on every call (returning `Ok(None)` only when not live or no data).
    /// For real ACC sources this reads raw shared-memory pages and parses
    /// them via `parse_raw_frame`, producing identical output to the
    /// `record-raw` + `parse-raw` CLI pipeline.
    ///
    /// Used by `run_recording_loop` for lossless recording.
    fn read_all_telemetry_frame(
        &mut self,
        sample_tick: u64,
        timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>>;
}

// ---------------------------------------------------------------------------
// AccTelemetrySource — real ACC shared-memory adapter
// ---------------------------------------------------------------------------

/// Wraps `AccSharedMemoryReader` as a `TelemetrySource`.
pub struct AccTelemetrySource {
    reader: AccSharedMemoryReader,
}

impl AccTelemetrySource {
    pub(crate) fn new(reader: AccSharedMemoryReader) -> Self {
        Self { reader }
    }

    /// Attempt to open ACC shared memory.
    pub(crate) fn open() -> TelemetryResult<Self> {
        AccSharedMemoryReader::open().map(Self::new)
    }
}

impl TelemetrySource for AccTelemetrySource {
    fn status(&mut self) -> TelemetryResult<AccGameStatus> {
        self.reader.status()
    }

    fn session_info(&mut self) -> TelemetryResult<AccSessionInfo> {
        Ok(self.reader.session_info())
    }

    fn read_static_bytes(&mut self) -> TelemetryResult<Vec<u8>> {
        self.reader.read_static_bytes()
    }

    fn read_telemetry_frame(
        &mut self,
        sample_tick: u64,
        timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>> {
        self.reader.read_telemetry_frame(sample_tick, timestamp_ns)
    }

    fn read_all_telemetry_frame(
        &mut self,
        sample_tick: u64,
        timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>> {
        let phys = self.reader.read_raw_physics()?;
        let graph = self.reader.read_raw_graphics()?;
        let frame = crate::parse_raw_frame(sample_tick, timestamp_ns, &phys, &graph)?;
        Ok(Some(frame))
    }
}

// ---------------------------------------------------------------------------
// ReplayTelemetrySource — replays frames from an acctlm2 file
// ---------------------------------------------------------------------------

/// Reads all frames from an acctlm2 telemetry file via `BinaryTelemetryReader`
/// and replays them one by one via `TelemetrySource`.
pub struct ReplayTelemetrySource {
    frames: VecDeque<TelemetryFrame>,
    #[allow(dead_code)]
    metadata: SessionMetadata,
    raw_static_bytes: Vec<u8>,
    session_info: AccSessionInfo,
}

impl ReplayTelemetrySource {
    /// Return the poll_hz stored in the session metadata.
    pub(crate) fn poll_hz(&self) -> f64 {
        self.metadata.poll_hz
    }

    /// Return a reference to the session metadata loaded from the file.
    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    /// Open an acctlm2 file and load all frames into memory.
    pub fn open(file_path: impl AsRef<std::path::Path>) -> TelemetryResult<Self> {
        let reader = BinaryTelemetryReader::open(&file_path)?;
        let frames: VecDeque<_> = reader.read_all_frames()?.into();
        let metadata = reader.metadata().clone();
        let raw_static_bytes = metadata.raw_static_bytes.clone();
        let session_info = AccSessionInfo {
            track_name: metadata.track_name.clone(),
            car_model: metadata.car_model.clone(),
        };
        Ok(Self {
            frames,
            metadata,
            raw_static_bytes,
            session_info,
        })
    }
}

impl TelemetrySource for ReplayTelemetrySource {
    fn status(&mut self) -> TelemetryResult<AccGameStatus> {
        if self.frames.is_empty() {
            Ok(AccGameStatus::Off)
        } else {
            Ok(AccGameStatus::Live)
        }
    }

    fn session_info(&mut self) -> TelemetryResult<AccSessionInfo> {
        Ok(self.session_info.clone())
    }

    fn read_static_bytes(&mut self) -> TelemetryResult<Vec<u8>> {
        Ok(self.raw_static_bytes.clone())
    }

    fn read_telemetry_frame(
        &mut self,
        sample_tick: u64,
        timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>> {
        let mut frame = match self.frames.pop_front() {
            Some(f) => f,
            None => return Ok(None),
        };
        frame.sample_tick = sample_tick;
        frame.timestamp_ns = timestamp_ns;
        Ok(Some(frame))
    }

    fn read_all_telemetry_frame(
        &mut self,
        sample_tick: u64,
        timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>> {
        let mut frame = match self.frames.pop_front() {
            Some(f) => f,
            None => return Ok(None),
        };
        frame.sample_tick = sample_tick;
        frame.timestamp_ns = timestamp_ns;
        Ok(Some(frame))
    }
}

// ---------------------------------------------------------------------------
// ScriptedTelemetrySource — fake source for testing
// ---------------------------------------------------------------------------

/// A single step in a scripted telemetry sequence.
#[derive(Debug)]
pub struct ScriptedStep {
    /// If set, overrides the current status.
    pub status: Option<AccGameStatus>,
    /// If set, the frame to deliver (subject to packet-id dedup).
    pub frame: Option<TelemetryFrame>,
    /// If set, the next `read_telemetry_frame` call returns this error.
    pub error: Option<TelemetryError>,
}

impl ScriptedStep {
    pub fn new() -> Self {
        Self {
            status: None,
            frame: None,
            error: None,
        }
    }

    pub fn with_status(mut self, s: AccGameStatus) -> Self {
        self.status = Some(s);
        self
    }

    pub fn with_frame(mut self, f: TelemetryFrame) -> Self {
        self.frame = Some(f);
        self
    }

    pub fn with_error(mut self, e: TelemetryError) -> Self {
        self.error = Some(e);
        self
    }
}

impl Default for ScriptedStep {
    fn default() -> Self {
        Self::new()
    }
}

/// A scripted telemetry source for testing.
///
/// Each call to `read_telemetry_frame()` advances through scripted steps.
/// Frames with the same `physics_packet_id` as the previous frame are
/// skipped (simulating real ACC packet dedup).
pub struct ScriptedTelemetrySource {
    steps: Vec<ScriptedStep>,
    index: usize,
    last_status: AccGameStatus,
    last_packet_id: i32,
    /// Whether an unrecoverable error has been set (simulates shmem disconnect)
    error_state: bool,
}

impl ScriptedTelemetrySource {
    /// Create a new scripted source with the given steps.
    pub fn new(steps: Vec<ScriptedStep>) -> Self {
        Self {
            steps,
            index: 0,
            last_status: AccGameStatus::Unavailable,
            last_packet_id: i32::MIN,
            error_state: false,
        }
    }

    /// Create an empty source (useful for testing status-only flows).
    pub fn empty() -> Self {
        Self::new(vec![])
    }

    fn current_step(&self) -> Option<&ScriptedStep> {
        self.steps.get(self.index)
    }
}

impl TelemetrySource for ScriptedTelemetrySource {
    fn status(&mut self) -> TelemetryResult<AccGameStatus> {
        if let Some(step) = self.current_step() {
            if let Some(s) = step.status {
                self.last_status = s;
            }
        }
        Ok(self.last_status)
    }

    fn session_info(&mut self) -> TelemetryResult<AccSessionInfo> {
        Ok(AccSessionInfo {
            track_name: "test_track".to_string(),
            car_model: "test_car".to_string(),
        })
    }

    fn read_static_bytes(&mut self) -> TelemetryResult<Vec<u8>> {
        Ok(Vec::new())
    }

    fn read_telemetry_frame(
        &mut self,
        _sample_tick: u64,
        _timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>> {
        // Check error state (persistent after first error)
        if self.error_state {
            return Err(TelemetryError::InvalidArgument(
                "scripted shared memory disconnected".to_string(),
            ));
        }

        if self.index >= self.steps.len() {
            return Ok(None);
        }

        // Check for upcoming error before advancing (to avoid clone)
        let has_error = self.current_step().and_then(|s| s.error.as_ref()).is_some();
        if has_error {
            self.error_state = true;
            self.index += 1;
            return Err(TelemetryError::InvalidArgument(
                "scripted shared memory disconnected".to_string(),
            ));
        }

        // Extract status and frame before advancing
        let step_status = self.current_step().and_then(|s| s.status);
        let step_frame = self.current_step().and_then(|s| s.frame.clone());

        // Advance to next step
        self.index += 1;

        // Check status (if step sets it)
        if let Some(s) = step_status {
            self.last_status = s;
        }

        // If not live, no frame
        if !self.last_status.is_live() {
            return Ok(None);
        }

        // Check frame packet-id dedup
        if let Some(ref frame) = step_frame {
            if frame.controls.physics_packet_id == self.last_packet_id {
                return Ok(None);
            }
            self.last_packet_id = frame.controls.physics_packet_id;
            return Ok(Some(frame.clone()));
        }

        Ok(None)
    }

    fn read_all_telemetry_frame(
        &mut self,
        _sample_tick: u64,
        _timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>> {
        // Same as read_telemetry_frame but without packet-ID dedup.
        // Every frame in the script is delivered regardless of packet_id.
        if self.error_state {
            return Err(TelemetryError::InvalidArgument(
                "scripted shared memory disconnected".to_string(),
            ));
        }

        if self.index >= self.steps.len() {
            return Ok(None);
        }

        let has_error = self.current_step().and_then(|s| s.error.as_ref()).is_some();
        if has_error {
            self.error_state = true;
            self.index += 1;
            return Err(TelemetryError::InvalidArgument(
                "scripted shared memory disconnected".to_string(),
            ));
        }

        let step_status = self.current_step().and_then(|s| s.status);
        let step_frame = self.current_step().and_then(|s| s.frame.clone());

        self.index += 1;

        if let Some(s) = step_status {
            self.last_status = s;
        }

        if !self.last_status.is_live() {
            return Ok(None);
        }

        if let Some(frame) = step_frame {
            return Ok(Some(frame));
        }

        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shmem::ACC_STATUS_LIVE;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
        PowertrainSample, SessionSample, TimingSample, TyreSample,
    };

    fn make_frame(seed: u64, speed: f32, status_val: i32) -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: seed,
            timestamp_ns: seed * 8_333_333,
            controls: ControlSample {
                sample_tick: seed,
                timestamp_ns: seed * 8_333_333,
                physics_packet_id: seed as i32,
                graphics_packet_id: 0,
                speed_kmh: speed,
                ..ControlSample::default()
            },
            motion: MotionSample::default(),
            tyres: TyreSample::default(),
            powertrain: PowertrainSample::default(),
            session: SessionSample {
                sample_tick: seed,
                timestamp_ns: seed * 8_333_333,
                status: status_val,
                ..SessionSample::default()
            },
            timing: TimingSample::default(),
            car_state: CarStateSample::default(),
            environment: EnvironmentSample::default(),
            other_cars: OtherCarsSample::default(),
        }
    }

    #[test]
    fn test_scripted_status_sequence() {
        // Simulate recording loop: status+frame per tick
        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(0, 10.0, 2)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(1, 20.0, 2)),
            ScriptedStep::new().with_status(AccGameStatus::Off),
        ];
        let mut src = ScriptedTelemetrySource::new(steps);

        // Tick 1: Live
        assert_eq!(src.status().unwrap(), AccGameStatus::Live);
        assert!(src.read_telemetry_frame(0, 0).unwrap().is_some());

        // Tick 2: Live
        assert_eq!(src.status().unwrap(), AccGameStatus::Live);
        assert!(src.read_telemetry_frame(1, 0).unwrap().is_some());

        // Tick 3: Off (no frame)
        assert_eq!(src.status().unwrap(), AccGameStatus::Off);
        assert!(src.read_telemetry_frame(2, 0).unwrap().is_none());
    }

    #[test]
    fn test_scripted_frame_delivery() {
        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(0, 100.0, 2)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(1, 150.0, 2)),
        ];
        let mut src = ScriptedTelemetrySource::new(steps);

        let f0 = src.read_telemetry_frame(0, 0).unwrap().unwrap();
        assert!((f0.controls.speed_kmh - 100.0).abs() < 0.01);

        let f1 = src.read_telemetry_frame(1, 0).unwrap().unwrap();
        assert!((f1.controls.speed_kmh - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_packet_id_dedup() {
        // Same packet ID across steps → second frame skipped
        let mut f0 = make_frame(0, 100.0, 2);
        f0.controls.physics_packet_id = 42;
        let mut f1 = make_frame(1, 200.0, 2);
        f1.controls.physics_packet_id = 42; // same packet ID

        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(f0),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(f1),
        ];
        let mut src = ScriptedTelemetrySource::new(steps);

        let r0 = src.read_telemetry_frame(0, 0).unwrap();
        assert!(r0.is_some()); // first frame delivered

        let r1 = src.read_telemetry_frame(1, 0).unwrap();
        assert!(r1.is_none()); // deduped — same packet_id
    }

    #[test]
    fn test_live_to_off_stops_frames() {
        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(0, 100.0, 2)),
            ScriptedStep::new().with_status(AccGameStatus::Off),
            ScriptedStep::new()
                .with_status(AccGameStatus::Off)
                .with_frame(make_frame(2, 200.0, 0)), // frame with Off status
        ];
        let mut src = ScriptedTelemetrySource::new(steps);

        // Live frame
        let f = src.read_telemetry_frame(0, 0).unwrap();
        assert!(f.is_some());

        // Status transitions to Off
        let _ = src.status();

        // After Off, no frame even if step has one
        let f2 = src.read_telemetry_frame(2, 0).unwrap();
        assert!(f2.is_none());
    }

    #[test]
    fn test_shmem_disconnect() {
        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(0, 100.0, 2)),
            ScriptedStep::new()
                .with_error(TelemetryError::InvalidArgument("shmem lost".to_string())),
        ];
        let mut src = ScriptedTelemetrySource::new(steps);

        // First frame ok
        assert!(src.read_telemetry_frame(0, 0).unwrap().is_some());

        // Second call returns error
        assert!(src.read_telemetry_frame(1, 0).is_err());

        // Subsequent calls also error (persistent)
        assert!(src.read_telemetry_frame(2, 0).is_err());
    }

    #[test]
    fn test_acc_telemetry_source_open_non_windows_returns_err() {
        // On non-Windows, open() should return Err
        #[cfg(not(windows))]
        {
            let result = AccTelemetrySource::open();
            assert!(result.is_err());
        }
    }

    // -----------------------------------------------------------------------
    // ReplayTelemetrySource tests
    // -----------------------------------------------------------------------

    /// Helper: create a test session metadata for replay tests.
    fn replay_test_metadata() -> SessionMetadata {
        let mut meta = SessionMetadata::new("monza", "porsche_911_gt3", 120.0);
        meta.raw_static_bytes = b"fake-static-page".to_vec();
        meta
    }

    /// Helper: create a single test frame with unique tick.
    fn replay_frame(idx: u64) -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: idx,
            timestamp_ns: idx * 8_333_333,
            controls: ControlSample {
                sample_tick: idx,
                timestamp_ns: idx * 8_333_333,
                physics_packet_id: idx as i32,
                speed_kmh: 50.0 + idx as f32 * 10.0,
                ..ControlSample::default()
            },
            session: SessionSample {
                status: ACC_STATUS_LIVE,
                ..SessionSample::default()
            },
            ..make_frame(idx, 0.0, 0)
        }
    }

    #[cfg(feature = "v2_writer")]
    #[test]
    fn test_replay_source_open_and_read_all_frames() {
        use crate::writer_v2::BinaryTelemetryWriterV2;
        use crate::writer::LiveTelemetryConfig;
        use std::sync::Arc;

        let tmp = std::env::temp_dir().join(format!("replay_test_{}.acctlm2", std::process::id()));
        let _ = std::fs::remove_file(&tmp); // clean slate

        let metadata = replay_test_metadata();
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&tmp, metadata.clone(), config)
                .expect("create_file");

        let frames_in: Vec<_> = (0u64..3).map(replay_frame).collect();
        for f in &frames_in {
            writer.write_frame(&Arc::new(f.clone())).expect("write_frame");
        }
        writer.finish().expect("finish");

        // Open replay source
        let mut source = ReplayTelemetrySource::open(&tmp).expect("open replay source");

        // Verify status is Live while frames remain
        assert_eq!(source.status().unwrap(), AccGameStatus::Live);

        // Verify session_info matches
        let info = source.session_info().unwrap();
        assert_eq!(info.track_name, "monza");
        assert_eq!(info.car_model, "porsche_911_gt3");

        // Verify read_static_bytes returns something (format-dependent round-trip)
        let _static_bytes = source.read_static_bytes().unwrap();

        // Read all frames — verify count and that sample_tick/timestamp_ns are overwritten
        for i in 0u64..3 {
            let frame = source
                .read_telemetry_frame(100 + i, 200 + i)
                .expect("read frame")
                .unwrap_or_else(|| panic!("expected frame {i}"));
            assert_eq!(frame.sample_tick, 100 + i);
            assert_eq!(frame.timestamp_ns, 200 + i);
            assert!((frame.controls.speed_kmh - (50.0 + i as f32 * 10.0)).abs() < 0.01);
        }

        // 4th read returns None
        assert!(source.read_telemetry_frame(999, 999).unwrap().is_none());

        // Status is now Off
        assert_eq!(source.status().unwrap(), AccGameStatus::Off);

        // Cleanup
        let _ = std::fs::remove_file(&tmp);
    }

    #[cfg(feature = "v2_writer")]
    #[test]
    fn test_replay_source_read_all_same_as_read_one() {
        use crate::writer_v2::BinaryTelemetryWriterV2;
        use crate::writer::LiveTelemetryConfig;
        use std::sync::Arc;

        let tmp =
            std::env::temp_dir().join(format!("replay_all_test_{}.acctlm2", std::process::id()));
        let _ = std::fs::remove_file(&tmp);

        let metadata = replay_test_metadata();
        let config = LiveTelemetryConfig::default();
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&tmp, metadata, config).expect("create_file");

        let frames_in: Vec<_> = (0u64..5).map(replay_frame).collect();
        for f in &frames_in {
            writer.write_frame(&Arc::new(f.clone())).expect("write_frame");
        }
        writer.finish().expect("finish");

        let mut source = ReplayTelemetrySource::open(&tmp).expect("open");

        // read_all_telemetry_frame should behave identically to read_telemetry_frame
        for i in 0u64..5 {
            let frame = source
                .read_all_telemetry_frame(10 + i, 20 + i)
                .expect("read_all frame")
                .unwrap_or_else(|| panic!("expected frame {i}"));
            assert_eq!(frame.sample_tick, 10 + i);
            assert_eq!(frame.timestamp_ns, 20 + i);
        }

        assert!(source.read_all_telemetry_frame(99, 99).unwrap().is_none());
        assert_eq!(source.status().unwrap(), AccGameStatus::Off);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_replay_source_invalid_file_returns_err() {
        let tmp = std::env::temp_dir().join("nonexistent_file_that_does_not_exist.acctlm2");
        let result = ReplayTelemetrySource::open(&tmp);
        assert!(result.is_err());
    }

    #[test]
    fn test_replay_source_status_live_when_frames() {
        // Direct construction — no file needed
        let mut source = ReplayTelemetrySource {
            frames: vec![replay_frame(0)].into(),
            metadata: replay_test_metadata(),
            raw_static_bytes: vec![],
            session_info: AccSessionInfo {
                track_name: "test".to_string(),
                car_model: "test".to_string(),
            },
        };

        assert_eq!(source.status().unwrap(), AccGameStatus::Live);

        let _ = source.read_telemetry_frame(0, 0).unwrap();
        assert_eq!(source.status().unwrap(), AccGameStatus::Off);
    }

    #[test]
    fn test_replay_source_empty_immediately_off() {
        let mut source = ReplayTelemetrySource {
            frames: VecDeque::new(),
            metadata: replay_test_metadata(),
            raw_static_bytes: vec![],
            session_info: AccSessionInfo::default(),
        };

        assert_eq!(source.status().unwrap(), AccGameStatus::Off);
        assert!(source.read_telemetry_frame(0, 0).unwrap().is_none());
        assert!(source.read_all_telemetry_frame(0, 0).unwrap().is_none());
    }
}
