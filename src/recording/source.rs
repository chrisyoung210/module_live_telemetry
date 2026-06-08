//! Telemetry source abstraction
//!
//! Defines `TelemetrySource` trait for injectable telemetry data.
//! Includes a real ACC shared-memory adapter and a scripted fake
//! for testing without real ACC shared memory.

use crate::error::{TelemetryError, TelemetryResult};
use crate::shmem::{AccGameStatus, AccSessionInfo, AccSharedMemoryReader};
use crate::writer::TelemetryFrame;

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
    pub fn new(reader: AccSharedMemoryReader) -> Self {
        Self { reader }
    }

    /// Attempt to open ACC shared memory.
    pub fn open() -> TelemetryResult<Self> {
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
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
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
            ScriptedStep::new()
                .with_status(AccGameStatus::Off),
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
            ScriptedStep::new()
                .with_status(AccGameStatus::Off),
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
}
