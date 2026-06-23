//! Shared recording engine — the core recording loop used by both
//! the CLI `record_command` and the `RecordingController` API.
//!
//! Extracted from `src/bin/acc-live-telemetry.rs:record_command()`.

use crate::distributor::TelemetryDistributor;
use crate::error::TelemetryResult;
use crate::recording::file_naming;
use crate::recording::source::TelemetrySource;
use crate::recording::status::{RecordingErrorKind, RecordingStatus, StatusSender, StopReason};
use crate::shmem::AccGameStatus;
use crate::types::RecordingSummary;
use crate::writer::LiveTelemetryConfig;
use crate::writer::TelemetryFrame;
use crate::writer_v2::BinaryTelemetryWriterV2;
use crate::SPageFileStatic;
use crossbeam_channel::Receiver;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Configuration for the recording engine.
#[derive(Clone)]
pub struct RecordingEngineConfig {
    /// Target polling frequency (Hz). Engine sleeps to match this rate.
    pub poll_hz: f64,
    /// Computed sleep duration between polls.
    pub poll_interval: Duration,
    /// Rows per chunk (passed to writer config).
    pub chunk_rows: usize,
    /// Interval for periodic status reporting (used by callers, not engine itself).
    pub status_interval: Duration,
    /// Optional periodic flush interval. `None` means flush only on finish.
    pub flush_interval: Option<Duration>,
}

/// Event fired when a lap completes during recording or dashboard.
#[derive(Debug, Clone)]
pub struct LapCompletedEvent {
    /// Lap number (1-based, 0 = out lap).
    pub lap_number: u32,
    /// Whether ACC considers this a valid lap.
    pub is_valid: bool,
    /// Lap time in milliseconds.
    pub lap_time_ms: i32,
    /// Whether this is an out lap (first lap from pits).
    pub is_out_lap: bool,
    /// Track name from ACC static info.
    pub track_name: String,
    /// All frames of the just-completed lap.
    /// Caller can use these to update a reference lap via
    /// `ComputeRegistry::replace_reference()`.
    pub lap_frames: Vec<TelemetryFrame>,
}

impl LapCompletedEvent {
    pub fn new(
        lap_number: u32,
        is_valid: bool,
        lap_time_ms: i32,
        is_out_lap: bool,
        track_name: String,
        lap_frames: Vec<TelemetryFrame>,
    ) -> Self {
        Self {
            lap_number,
            is_valid,
            lap_time_ms,
            is_out_lap,
            track_name,
            lap_frames,
        }
    }
}

/// Callback invoked when a lap completes.
pub type LapCompletedCallback = Box<dyn Fn(LapCompletedEvent) + Send>;

/// Result returned by `run_recording_loop` after the recording finishes.
#[derive(Debug, Clone)]
pub struct RecordingLoopResult {
    /// Summary statistics from the writer.
    pub summary: RecordingSummary,
    /// Full path to the output file, if recording actually started.
    pub output_path: Option<PathBuf>,
    /// Whether the loop was cancelled before recording started (no writer created).
    pub cancelled_before_start: bool,
    /// Why this single-session loop stopped.
    pub stop_reason: StopReason,
}

/// Run the recording loop to completion.
///
/// This is the shared core shared by the CLI `record_command` and
/// `RecordingController`. It:
///
/// 1. Waits for ACC to go Live, then creates the writer
/// 2. Reads frames, writes them to disk, optionally distributes to dashboard
/// 3. Detects lap completions and fires `lap_completed` callback
/// 4. Periodically flushes
/// 5. Auto-stops on session end (Live→Off) or shared-memory disconnect
/// 6. Stops on `stop_rx` signal
/// 7. Finishes the writer and returns `RecordingLoopResult`
///
/// The caller is responsible for:
/// - Setting up the `TelemetrySource` (real ACC or fake)
/// - Setting up dashboard consumers before calling (via `dashboard_distributor`)
/// - Reading `stop_rx` for manual stop signals
pub fn run_recording_loop(
    mut config: RecordingEngineConfig,
    source: &mut impl TelemetrySource,
    output_dir: PathBuf,
    dashboard_distributor: Option<&TelemetryDistributor>,
    stop_rx: Receiver<()>,
    lap_completed: Option<&LapCompletedCallback>,
    status_tx: Option<&StatusSender>,
    session_meta: Option<&Arc<Mutex<Option<crate::types::SessionMetadata>>>>,
) -> TelemetryResult<RecordingLoopResult> {
    // Clamp poll_hz
    config.poll_hz = config.poll_hz.max(1.0);
    config.poll_interval = Duration::from_secs_f64(1.0 / config.poll_hz);

    // --- Phase 1: Wait for Live ---
    let mut writer: Option<BinaryTelemetryWriterV2> = None;
    let mut sample_tick = 0u64;
    let mut recording_started_at = Instant::now();
    let mut last_status: Option<AccGameStatus> = None;
    let mut last_flush = Instant::now();
    let mut output_path: Option<PathBuf> = None;

    // Lap completion detection state
    let mut _prev_norm_pos: Option<f32> = None;
    let mut last_completed_laps: Option<u32> = None;
    let mut current_lap_frames: Vec<TelemetryFrame> = Vec::new();
    let mut current_lap_is_valid = true;
    let mut recording_track_name: Option<String> = None;

    loop {
        let tick_start = Instant::now();

        // Check for stop signal
        if stop_rx.try_recv().is_ok() {
            if let Some(w) = writer.take() {
                let summary = w.finish()?;
                return Ok(RecordingLoopResult {
                    summary,
                    output_path,
                    cancelled_before_start: false,
                    stop_reason: StopReason::Manual,
                });
            }
            return Ok(RecordingLoopResult {
                summary: RecordingSummary {
                    total_samples: 0,
                    chunk_count: 0,
                    total_bytes: 0,
                    footer_offset: 0,
                    duration: Duration::from_secs(0),
                },
                output_path: None,
                cancelled_before_start: true,
                stop_reason: StopReason::Manual,
            });
        }

        // Try to connect / read status
        let status = match source.status() {
            Ok(s) => s,
            Err(_err) => {
                // Shared memory error — if already recording, finish and exit
                if let Some(w) = writer.take() {
                    let summary = w.finish()?;
                    return Ok(RecordingLoopResult {
                        summary,
                        output_path,
                        cancelled_before_start: false,
                        stop_reason: StopReason::ShmemLost,
                    });
                }
                // Not recording yet — retry after sleep
                return Ok(RecordingLoopResult {
                    summary: RecordingSummary {
                        total_samples: 0,
                        chunk_count: 0,
                        total_bytes: 0,
                        footer_offset: 0,
                        duration: Duration::from_secs(0),
                    },
                    output_path: None,
                    cancelled_before_start: true,
                    stop_reason: StopReason::ShmemLost,
                });
            }
        };

        // Track status changes (for caller logging)
        let resumed_from_pause = status.is_live() && last_status == Some(AccGameStatus::Pause);
        let status_changed = Some(status) != last_status;
        if status_changed {
            last_status = Some(status);
        }

        if status.is_live() {
            // --- Phase 2: Recording ---
            if writer.is_none() {
                // Read session info to get metadata
                let session = source.session_info()?;
                recording_track_name = Some(session.track_name.clone());
                let path = file_naming::build_unique_output_path(
                    &output_dir,
                    &session.track_name,
                    &session.car_model,
                )?;
                let mut metadata = crate::SessionMetadata::new(
                    session.track_name,
                    session.car_model,
                    config.poll_hz,
                );
                // Populate extra metadata from static page (matching record CLI behavior)
                if let Ok(static_bytes) = source.read_static_bytes() {
                    let stat = SPageFileStatic::from_raw(&static_bytes);
                    metadata.sm_version = stat.sm_version_str();
                    metadata.ac_version = stat.ac_version_str();
                    metadata.number_of_sessions = stat.number_of_sessions;
                    metadata.num_cars = stat.num_cars;
                    metadata.sector_count = stat.sector_count;
                    metadata.max_rpm = stat.max_rpm;
                    metadata.max_torque = stat.max_torque;
                    metadata.max_power = stat.max_power;
                    metadata.max_fuel = stat.max_fuel;
                    metadata.penalties_enabled = stat.penalties_enabled;
                    metadata.raw_static_bytes = static_bytes;
                }

                // Store session metadata for m1 query BEFORE sending RecordingStarted
                if let Some(sm) = session_meta {
                    if let Ok(mut guard) = sm.lock() {
                        *guard = Some(metadata.clone());
                    }
                }

                let w_config = LiveTelemetryConfig {
                    poll_hz: config.poll_hz,
                    chunk_rows: config.chunk_rows,
                };
                writer = Some(BinaryTelemetryWriterV2::create_file(
                    &path, metadata, w_config,
                )?);
                output_path = Some(path);
                sample_tick = 0;
                recording_started_at = Instant::now();
                send_status(status_tx, RecordingStatus::RecordingStarted);
            }

            if resumed_from_pause && writer.is_some() {
                let elapsed = recording_started_at.elapsed();
                let bytes_written = output_path
                    .as_ref()
                    .and_then(|path| std::fs::metadata(path).ok())
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                let fps = if elapsed.is_zero() {
                    0.0
                } else {
                    sample_tick as f64 / elapsed.as_secs_f64()
                };
                send_status(
                    status_tx,
                    RecordingStatus::Running {
                        sample_count: sample_tick,
                        bytes_written,
                        elapsed,
                        fps,
                    },
                );
            }

            let timestamp_ns = recording_started_at
                .elapsed()
                .as_nanos()
                .min(u64::MAX as u128) as u64;

            match source.read_all_telemetry_frame(sample_tick, timestamp_ns) {
                Ok(Some(frame)) => {
                    let frame_arc = Arc::new(frame);

                    // --- Lap completion detection ---
                    if let Some(cb) = lap_completed {
                        let completed_laps = frame_arc.session.completed_laps as u32;
                        if last_completed_laps.is_some_and(|previous| completed_laps > previous) {
                            let lap_time_ms = frame_arc.timing.i_last_time;
                            let lap_frames = std::mem::take(&mut current_lap_frames);

                            cb(LapCompletedEvent::new(
                                completed_laps,
                                current_lap_is_valid,
                                lap_time_ms,
                                false,
                                recording_track_name
                                    .clone()
                                    .unwrap_or_else(|| "unknown_track".to_string()),
                                lap_frames,
                            ));
                            current_lap_is_valid = frame_arc.session.is_valid_lap != 0;
                        } else {
                            current_lap_is_valid &= frame_arc.session.is_valid_lap != 0;
                        }
                        current_lap_frames.push((*frame_arc).clone());
                        last_completed_laps = Some(completed_laps);
                    }

                    // Distribute to dashboard (if active)
                    if let Some(dist) = dashboard_distributor {
                        dist.distribute(Arc::clone(&frame_arc));
                    }

                    // Write to file
                    if let Some(w) = writer.as_mut() {
                        w.write_frame(&frame_arc)?;
                    }

                    sample_tick = sample_tick.saturating_add(1);
                }
                Ok(None) => {
                    // No new frame — normal, ACC packet ID unchanged or not live
                }
                Err(err) => {
                    // Read error — finish and exit
                    if let Some(w) = writer.take() {
                        let summary = w.finish()?;
                        return Ok(RecordingLoopResult {
                            summary,
                            output_path,
                            cancelled_before_start: false,
                            stop_reason: StopReason::ShmemLost,
                        });
                    }
                    // No writer yet — propagate the error
                    return Err(err);
                }
            }
        } else if status.is_pause() {
            // Pause: keep writer open, suspend sampling
            if writer.is_some() && status_changed {
                send_status(status_tx, RecordingStatus::Paused);
            }
        } else {
            // Session ended (Off, Replay, etc.)
            if let Some(w) = writer.take() {
                let summary = w.finish()?;
                return Ok(RecordingLoopResult {
                    summary,
                    output_path,
                    cancelled_before_start: false,
                    stop_reason: StopReason::SessionEnd,
                });
            }
        }

        // Periodic flush
        if let Some(flush_interval) = config.flush_interval {
            if last_flush.elapsed() >= flush_interval {
                if let Some(ref mut w) = writer {
                    w.flush()?;
                }
                last_flush = Instant::now();
            }
        }

        // Sleep to maintain poll rate
        sleep_remaining(tick_start, config.poll_interval);
    }
}

/// Run a replay loop that replays frames from a `TelemetrySource` without writing any files.
///
/// This is the "no file writing" version of `run_recording_loop`. It:
///
/// 1. Sends `ReplayStarted` status
/// 2. Polls the source every `poll_interval` (scaled by `speed_multiplier`)
/// 3. Detects lap completions and fires `lap_completed` callback
/// 4. Distributes frames to the dashboard via `dashboard_distributor`
/// 5. Auto-stops when frames are exhausted (`source.status()` returns `Off`)
/// 6. Stops on `stop_rx` signal
/// 7. Returns the `StopReason`
///
/// This function NEVER creates a `BinaryTelemetryWriterV2` or writes any file.
pub fn run_replay_loop(
    speed_multiplier: f64,
    source: &mut impl TelemetrySource,
    poll_hz: f64,
    dashboard_distributor: Option<&TelemetryDistributor>,
    stop_rx: Receiver<()>,
    lap_completed: Option<&LapCompletedCallback>,
    status_tx: Option<&StatusSender>,
) -> TelemetryResult<StopReason> {
    // Clamp poll_hz and compute poll_interval with speed multiplier
    let poll_hz = poll_hz.max(1.0);
    let poll_interval =
        Duration::from_secs_f64(1.0 / (poll_hz * speed_multiplier.max(0.001)));

    let mut sample_tick = 0u64;
    let replay_start = Instant::now();

    // Lap completion detection state (matches run_recording_loop)
    let mut last_completed_laps: Option<u32> = None;
    let mut current_lap_frames: Vec<TelemetryFrame> = Vec::new();
    let mut current_lap_is_valid = true;
    let replay_track_name = source
        .session_info()
        .map(|s| s.track_name)
        .unwrap_or_else(|_| "unknown_track".to_string());

    // Status: ReplayStarted
    send_status(status_tx, RecordingStatus::ReplayStarted);

    loop {
        let tick_start = Instant::now();

        // 1. Check stop signal
        if stop_rx.try_recv().is_ok() {
            send_status(
                status_tx,
                RecordingStatus::Stopping {
                    reason: StopReason::Manual,
                },
            );
            return Ok(StopReason::Manual);
        }

        // 2. Check source status — Off means frames exhausted
        let status = source.status()?;
        if status == AccGameStatus::Off {
            send_status(
                status_tx,
                RecordingStatus::Stopping {
                    reason: StopReason::FramesExhausted,
                },
            );
            return Ok(StopReason::FramesExhausted);
        }

        // 3. Compute timestamp_ns from elapsed since replay start
        let timestamp_ns = replay_start.elapsed().as_nanos().min(u64::MAX as u128) as u64;

        // 4. Read frame
        match source.read_all_telemetry_frame(sample_tick, timestamp_ns) {
            Ok(Some(frame)) => {
                let frame_arc = Arc::new(frame);

                // --- Lap completion detection (matching engine.rs:273-291) ---
                if let Some(cb) = lap_completed {
                    let completed_laps = frame_arc.session.completed_laps as u32;
                    if last_completed_laps
                        .is_some_and(|previous| completed_laps > previous)
                    {
                        let lap_time_ms = frame_arc.timing.i_last_time;
                        let lap_frames = std::mem::take(&mut current_lap_frames);

                        cb(LapCompletedEvent::new(
                            completed_laps,
                            current_lap_is_valid,
                            lap_time_ms,
                            false,
                            replay_track_name.clone(),
                            lap_frames,
                        ));
                        current_lap_is_valid = frame_arc.session.is_valid_lap != 0;
                    } else {
                        current_lap_is_valid &= frame_arc.session.is_valid_lap != 0;
                    }
                    current_lap_frames.push((*frame_arc).clone());
                    last_completed_laps = Some(completed_laps);
                }

                // Dashboard distribute (matching engine.rs:295-297)
                if let Some(dist) = dashboard_distributor {
                    dist.distribute(Arc::clone(&frame_arc));
                }

                sample_tick = sample_tick.saturating_add(1);

                // Periodic Running status
                let elapsed = replay_start.elapsed();
                let fps = if elapsed.is_zero() {
                    0.0
                } else {
                    sample_tick as f64 / elapsed.as_secs_f64()
                };
                send_status(
                    status_tx,
                    RecordingStatus::Running {
                        sample_count: sample_tick,
                        bytes_written: 0,
                        elapsed,
                        fps,
                    },
                );
            }
            Ok(None) => {
                // No more frames available
                send_status(
                    status_tx,
                    RecordingStatus::Stopping {
                        reason: StopReason::FramesExhausted,
                    },
                );
                return Ok(StopReason::FramesExhausted);
            }
            Err(err) => {
                send_status(
                    status_tx,
                    RecordingStatus::Error {
                        message: err.to_string(),
                        kind: RecordingErrorKind::ShmemDisconnected,
                    },
                );
                return Err(err);
            }
        }

        // 5. Sleep to maintain poll rate
        sleep_remaining(tick_start, poll_interval);
    }
}

/// Sleep for the remaining time in the current tick to maintain target poll rate.
fn sleep_remaining(tick_start: Instant, poll_interval: Duration) {
    let elapsed = tick_start.elapsed();
    if elapsed < poll_interval {
        std::thread::sleep(poll_interval - elapsed);
    }
}

fn send_status(status_tx: Option<&StatusSender>, status: RecordingStatus) {
    if let Some(tx) = status_tx {
        let _ = tx.try_send(status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::source::{ScriptedStep, ScriptedTelemetrySource};
    use crate::shmem::AccGameStatus;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
        PowertrainSample, SessionSample, TimingSample, TyreSample,
    };
    use crate::writer::TelemetryFrame;
    use crossbeam_channel::bounded;
    use std::collections::VecDeque;
    use std::time::Duration;

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

    fn make_lap_frame(
        seed: u64,
        completed_laps: i32,
        normalized_position: f32,
        current_time_ms: i32,
        last_time_ms: i32,
    ) -> TelemetryFrame {
        let mut frame = make_frame(seed, 100.0, 2);
        frame.session.completed_laps = completed_laps;
        frame.session.normalized_car_position = normalized_position;
        frame.session.is_valid_lap = 1;
        frame.timing.i_current_time = current_time_ms;
        frame.timing.i_last_time = last_time_ms;
        frame
    }

    struct PauseResumeSource {
        statuses: VecDeque<AccGameStatus>,
        current_status: AccGameStatus,
        frame_seed: u64,
    }

    impl PauseResumeSource {
        fn new(statuses: impl IntoIterator<Item = AccGameStatus>) -> Self {
            Self {
                statuses: statuses.into_iter().collect(),
                current_status: AccGameStatus::Unavailable,
                frame_seed: 0,
            }
        }

        fn next_frame(&mut self) -> Option<TelemetryFrame> {
            if !self.current_status.is_live() {
                return None;
            }
            let seed = self.frame_seed;
            self.frame_seed += 1;
            Some(make_frame(seed, 100.0 + seed as f32, 2))
        }
    }

    impl TelemetrySource for PauseResumeSource {
        fn status(&mut self) -> TelemetryResult<AccGameStatus> {
            if let Some(status) = self.statuses.pop_front() {
                self.current_status = status;
            }
            Ok(self.current_status)
        }

        fn session_info(&mut self) -> TelemetryResult<crate::shmem::AccSessionInfo> {
            Ok(crate::shmem::AccSessionInfo {
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
            Ok(self.next_frame())
        }

        fn read_all_telemetry_frame(
            &mut self,
            _sample_tick: u64,
            _timestamp_ns: u64,
        ) -> TelemetryResult<Option<TelemetryFrame>> {
            Ok(self.next_frame())
        }
    }

    #[test]
    fn test_engine_emits_running_when_pause_resumes_live() -> TelemetryResult<()> {
        let dir = std::env::temp_dir().join(format!(
            "test_engine_resume_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir)?;
        let mut source = PauseResumeSource::new([
            AccGameStatus::Live,
            AccGameStatus::Pause,
            AccGameStatus::Live,
            AccGameStatus::Off,
        ]);
        let config = RecordingEngineConfig {
            poll_hz: 1000.0,
            poll_interval: Duration::from_millis(1),
            chunk_rows: 256,
            status_interval: Duration::from_secs(1),
            flush_interval: None,
        };
        let (_stop_tx, stop_rx) = bounded::<()>(1);
        let (status_tx, status_rx) = bounded(8);

        let result = run_recording_loop(
            config,
            &mut source,
            dir.clone(),
            None,
            stop_rx,
            None,
            Some(&status_tx),
            None,
        )?;
        let statuses: Vec<_> = status_rx.try_iter().collect();
        assert!(matches!(statuses[0], RecordingStatus::RecordingStarted));
        assert!(matches!(statuses[1], RecordingStatus::Paused));
        assert!(matches!(
            statuses[2],
            RecordingStatus::Running {
                sample_count: 1,
                ..
            }
        ));

        if let Some(path) = result.output_path {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir(&dir);
        Ok(())
    }

    #[test]
    fn test_engine_records_fake_source() -> TelemetryResult<()> {
        let dir = std::env::temp_dir().join(format!(
            "test_engine_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir)?;

        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(0, 100.0, 2)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(1, 150.0, 2)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(2, 200.0, 2)),
            ScriptedStep::new().with_status(AccGameStatus::Off),
        ];
        let mut src = ScriptedTelemetrySource::new(steps);

        let config = RecordingEngineConfig {
            poll_hz: 120.0,
            poll_interval: Duration::from_secs_f64(1.0 / 120.0),
            chunk_rows: 256,
            status_interval: Duration::from_secs(2),
            flush_interval: None,
        };

        let (_stop_tx, stop_rx) = bounded::<()>(1);

        let result = run_recording_loop(config, &mut src, dir.clone(), None, stop_rx, None, None, None)?;
        let output_path = result.output_path.as_ref().expect("recording output path");

        assert_eq!(result.summary.total_samples, 3);
        assert_eq!(result.stop_reason, StopReason::SessionEnd);
        assert!(output_path.starts_with(&dir));
        assert!(output_path.exists());
        let output_name = output_path.to_string_lossy();
        assert!(output_name.contains("test_car"));
        assert!(output_name.contains("test_track"));

        // Verify file is readable via BinaryTelemetryReaderV2
        let reader = crate::reader_v2::BinaryTelemetryReaderV2::open(output_path)?;
        assert_eq!(reader.summary().total_samples, 3);

        // Cleanup
        let _ = std::fs::remove_file(output_path);
        let _ = std::fs::remove_dir(&dir);
        Ok(())
    }

    #[test]
    fn test_first_completed_valid_lap_is_emitted_as_reference_candidate() -> TelemetryResult<()> {
        let dir = std::env::temp_dir().join(format!(
            "test_engine_first_lap_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir)?;

        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_lap_frame(0, 0, 0.10, 10_000, 0)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_lap_frame(1, 0, 0.90, 80_000, 0)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_lap_frame(2, 1, 0.01, 500, 90_000)),
            ScriptedStep::new().with_status(AccGameStatus::Off),
        ];
        let mut source = ScriptedTelemetrySource::new(steps);
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let callback_events = Arc::clone(&events);
        let callback: LapCompletedCallback = Box::new(move |event| {
            callback_events.lock().unwrap().push(event);
        });
        let config = RecordingEngineConfig {
            poll_hz: 1000.0,
            poll_interval: Duration::from_millis(1),
            chunk_rows: 256,
            status_interval: Duration::from_secs(1),
            flush_interval: None,
        };
        let (_stop_tx, stop_rx) = bounded::<()>(1);

        let result = run_recording_loop(
            config,
            &mut source,
            dir.clone(),
            None,
            stop_rx,
            Some(&callback),
            None,
            None,
        )?;

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].lap_number, 1);
        assert!(events[0].is_valid);
        assert!(!events[0].is_out_lap);
        assert_eq!(events[0].lap_time_ms, 90_000);
        assert_eq!(events[0].lap_frames.len(), 2);
        assert_eq!(
            events[0].lap_frames[1].session.normalized_car_position,
            0.90
        );
        drop(events);

        if let Some(path) = result.output_path {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir(&dir);
        Ok(())
    }

    #[test]
    fn test_engine_stop_signal() -> TelemetryResult<()> {
        let dir = std::env::temp_dir().join(format!(
            "test_engine_stop_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir)?;

        // Many Live frames, but we'll send stop signal
        let mut steps: Vec<ScriptedStep> = (0..100)
            .map(|i| {
                ScriptedStep::new()
                    .with_status(AccGameStatus::Live)
                    .with_frame(make_frame(i, 100.0 + i as f32, 2))
            })
            .collect();
        // Append an Off at end so if stop doesn't work, it still terminates
        steps.push(ScriptedStep::new().with_status(AccGameStatus::Off));

        let mut src = ScriptedTelemetrySource::new(steps);

        let config = RecordingEngineConfig {
            poll_hz: 1000.0, // fast poll
            poll_interval: Duration::from_millis(1),
            chunk_rows: 256,
            status_interval: Duration::from_secs(2),
            flush_interval: None,
        };

        let (stop_tx, stop_rx) = bounded::<()>(1);

        // Send stop signal in a separate thread after a short delay
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let _ = stop_tx.send(());
        });

        let result = run_recording_loop(config, &mut src, dir.clone(), None, stop_rx, None, None, None)?;
        handle.join().unwrap();

        // Should have recorded some frames before stopping
        assert!(result.summary.total_samples > 0);
        assert!(result.summary.total_samples < 100); // stopped early
        assert_eq!(result.stop_reason, StopReason::Manual);
        let output_path = result.output_path.as_ref().expect("recording output path");
        assert!(output_path.starts_with(&dir));
        assert!(output_path.exists());

        // Cleanup
        let _ = std::fs::remove_file(output_path);
        let _ = std::fs::remove_dir(&dir);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // run_replay_loop tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_replay_loop_basic() -> TelemetryResult<()> {
        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(0, 100.0, 2)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(1, 150.0, 2)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(2, 200.0, 2)),
            ScriptedStep::new().with_status(AccGameStatus::Off),
        ];
        let mut source = ScriptedTelemetrySource::new(steps);

        let (_stop_tx, stop_rx) = bounded::<()>(1);
        let (status_tx, status_rx) = bounded(16);

        let result = run_replay_loop(
            1.0,
            &mut source,
            120.0,
            None,
            stop_rx,
            None,
            Some(&status_tx),
        )?;

        assert_eq!(result, StopReason::FramesExhausted);

        let statuses: Vec<_> = status_rx.try_iter().collect();
        assert!(matches!(statuses[0], RecordingStatus::ReplayStarted));
        assert!(matches!(
            statuses.last().unwrap(),
            RecordingStatus::Stopping {
                reason: StopReason::FramesExhausted
            }
        ));

        Ok(())
    }

    #[test]
    fn test_replay_loop_lap_callback() -> TelemetryResult<()> {
        let steps = vec![
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_lap_frame(0, 0, 0.10, 10_000, 0)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_lap_frame(1, 0, 0.90, 80_000, 0)),
            ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_lap_frame(2, 1, 0.01, 500, 90_000)),
            ScriptedStep::new().with_status(AccGameStatus::Off),
        ];
        let mut source = ScriptedTelemetrySource::new(steps);

        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let callback_events = Arc::clone(&events);
        let callback: LapCompletedCallback = Box::new(move |event| {
            callback_events.lock().unwrap().push(event);
        });

        let (_stop_tx, stop_rx) = bounded::<()>(1);
        let (status_tx, _status_rx) = bounded(16);

        let result = run_replay_loop(
            1.0,
            &mut source,
            120.0,
            None,
            stop_rx,
            Some(&callback),
            Some(&status_tx),
        )?;

        assert_eq!(result, StopReason::FramesExhausted);

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].lap_number, 1);
        assert!(events[0].is_valid);
        assert!(!events[0].is_out_lap);
        assert_eq!(events[0].lap_time_ms, 90_000);
        assert_eq!(events[0].lap_frames.len(), 2);

        Ok(())
    }

    #[test]
    fn test_replay_loop_stop_signal() -> TelemetryResult<()> {
        // Many frames to keep the loop running until the stop signal arrives
        let mut steps: Vec<ScriptedStep> = (0..100)
            .map(|i| {
                ScriptedStep::new()
                    .with_status(AccGameStatus::Live)
                    .with_frame(make_frame(i, 100.0 + i as f32, 2))
            })
            .collect();
        // Fallback Off so the loop terminates even if the stop signal is missed
        steps.push(ScriptedStep::new().with_status(AccGameStatus::Off));

        let mut source = ScriptedTelemetrySource::new(steps);

        let (stop_tx, stop_rx) = bounded::<()>(1);
        let (status_tx, _status_rx) = bounded(16);

        // Send stop signal in a separate thread after a short delay
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let _ = stop_tx.send(());
        });

        let result = run_replay_loop(
            1.0,
            &mut source,
            1000.0,
            None,
            stop_rx,
            None,
            Some(&status_tx),
        )?;

        handle.join().unwrap();
        assert_eq!(result, StopReason::Manual);

        Ok(())
    }

    #[test]
    fn test_replay_loop_speed_multiplier() -> TelemetryResult<()> {
        // Verify that speed_multiplier = 10x runs significantly faster than 1x
        let make_steps = || -> Vec<ScriptedStep> {
            let mut steps: Vec<ScriptedStep> = (0..5)
                .map(|i| {
                    ScriptedStep::new()
                        .with_status(AccGameStatus::Live)
                        .with_frame(make_frame(i, 100.0, 2))
                })
                .collect();
            steps.push(ScriptedStep::new().with_status(AccGameStatus::Off));
            steps
        };

        // Run at speed 1x
        let mut source_1x = ScriptedTelemetrySource::new(make_steps());
        let (_stop_tx, stop_rx_1x) = bounded::<()>(1);
        let (status_tx_1x, _status_rx) = bounded(16);
        let start_1x = Instant::now();
        run_replay_loop(
            1.0,
            &mut source_1x,
            5.0,
            None,
            stop_rx_1x,
            None,
            Some(&status_tx_1x),
        )?;
        let duration_1x = start_1x.elapsed();

        // Run at speed 10x
        let mut source_10x = ScriptedTelemetrySource::new(make_steps());
        let (_stop_tx, stop_rx_10x) = bounded::<()>(1);
        let (status_tx_10x, _status_rx) = bounded(16);
        let start_10x = Instant::now();
        run_replay_loop(
            10.0,
            &mut source_10x,
            5.0,
            None,
            stop_rx_10x,
            None,
            Some(&status_tx_10x),
        )?;
        let duration_10x = start_10x.elapsed();

        // 10x should be at least 2x faster than 1x (very generous margin)
        assert!(
            duration_10x.as_millis() < duration_1x.as_millis() / 2,
            "Expected 10x replay to be much faster than 1x. 1x={:?}, 10x={:?}",
            duration_1x,
            duration_10x,
        );

        Ok(())
    }
}
