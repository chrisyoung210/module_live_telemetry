//! Shared recording engine — the core recording loop used by both
//! the CLI `record_command` and the `RecordingController` API.
//!
//! Extracted from `src/bin/acc-live-telemetry.rs:record_command()`.

use crate::error::{TelemetryError, TelemetryResult};
use crate::recording::source::TelemetrySource;
use crate::shmem::AccGameStatus;
use crate::types::RecordingSummary;
use crate::writer::{BinaryTelemetryWriter, LiveTelemetryConfig};
use crate::distributor::TelemetryDistributor;
use crate::writer::TelemetryFrame;
use crossbeam_channel::Receiver;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
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
        lap_frames: Vec<TelemetryFrame>,
    ) -> Self {
        Self { lap_number, is_valid, lap_time_ms, is_out_lap, lap_frames }
    }
}

/// Callback invoked when a lap completes.
pub type LapCompletedCallback = Box<dyn Fn(LapCompletedEvent) + Send>;

/// Result returned by `run_recording_loop` after the recording finishes.
#[derive(Debug, Clone)]
pub struct RecordingLoopResult {
    /// Summary statistics from the writer.
    pub summary: RecordingSummary,
    /// Full path to the output file.
    pub output_path: PathBuf,
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
    output_path: PathBuf,
    dashboard_distributor: Option<&TelemetryDistributor>,
    stop_rx: Receiver<()>,
    lap_completed: Option<LapCompletedCallback>,
) -> TelemetryResult<RecordingLoopResult> {
    // Clamp poll_hz
    config.poll_hz = config.poll_hz.max(1.0);
    config.poll_interval = Duration::from_secs_f64(1.0 / config.poll_hz);

    // --- Phase 1: Wait for Live ---
    let mut writer: Option<BinaryTelemetryWriter<File>> = None;
    let mut sample_tick = 0u64;
    let mut recording_started_at = Instant::now();
    let mut last_flush = Instant::now();
    let mut last_status: Option<AccGameStatus> = None;

    // Lap completion detection state
    let mut _prev_norm_pos: Option<f32> = None;
    let mut lap_completed_count: u32 = 0;
    let mut last_completed_laps: u32 = 0;
    let mut current_lap_frames: Vec<TelemetryFrame> = Vec::new();

    loop {
        let tick_start = Instant::now();

        // Check for stop signal
        if stop_rx.try_recv().is_ok() {
            if let Some(w) = writer.take() {
                let (_, summary) = w.finish()?;
                return Ok(RecordingLoopResult {
                    summary,
                    output_path,
                });
            }
            return Err(TelemetryError::InvalidArgument(
                "stop requested before recording started".to_string(),
            ));
        }

        // Try to connect / read status
        let status = match source.status() {
            Ok(s) => s,
            Err(_err) => {
                // Shared memory error — if already recording, finish and exit
                if let Some(w) = writer.take() {
                    let (_, summary) = w.finish()?;
                    return Ok(RecordingLoopResult {
                        summary,
                        output_path,
                    });
                }
                // Not recording yet — retry after sleep
                sleep_remaining(tick_start, config.poll_interval);
                continue;
            }
        };

        // Track status changes (for caller logging)
        if Some(status) != last_status {
            last_status = Some(status);
        }

        if status.is_live() {
            // --- Phase 2: Recording ---
            if writer.is_none() {
                // Read session info to get metadata
                let session = source.session_info()?;
                let metadata = crate::SessionMetadata::new(
                    session.track_name,
                    session.car_model,
                    config.poll_hz,
                );
                let w_config = LiveTelemetryConfig {
                    poll_hz: config.poll_hz,
                    chunk_rows: config.chunk_rows,
                };
                writer = Some(BinaryTelemetryWriter::create_file(
                    &output_path,
                    metadata,
                    w_config,
                )?);
                sample_tick = 0;
                recording_started_at = Instant::now();
                last_flush = Instant::now();
            }

            let timestamp_ns = recording_started_at
                .elapsed()
                .as_nanos()
                .min(u64::MAX as u128) as u64;

            match source.read_telemetry_frame(sample_tick, timestamp_ns) {
                Ok(Some(frame)) => {
                    let frame_arc = Arc::new(frame);

                    // --- Lap completion detection ---
                    if let Some(ref cb) = lap_completed {
                        let completed_laps = frame_arc.session.completed_laps as u32;
                        current_lap_frames.push((*frame_arc).clone());
                        if completed_laps > last_completed_laps {
                            let is_out_lap = lap_completed_count == 0;
                            let lap_time_ms = frame_arc.timing.i_last_time;
                            let is_valid = completed_laps > 1 || frame_arc.session.is_valid_lap != 0;
                            let lap_frames = std::mem::take(&mut current_lap_frames);

                            cb(LapCompletedEvent::new(
                                lap_completed_count,
                                is_valid,
                                lap_time_ms,
                                is_out_lap,
                                lap_frames,
                            ));
                            lap_completed_count += 1;
                        }
                        last_completed_laps = completed_laps;
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
                        let (_, summary) = w.finish()?;
                        return Ok(RecordingLoopResult {
                            summary,
                            output_path,
                        });
                    }
                    // No writer yet — propagate the error
                    return Err(err);
                }
            }
        } else if status.is_pause() {
            // Pause: keep writer open, suspend sampling
        } else {
            // Session ended (Off, Replay, etc.)
            if let Some(w) = writer.take() {
                let (_, summary) = w.finish()?;
                return Ok(RecordingLoopResult {
                    summary,
                    output_path,
                });
            }
        }

        // Periodic flush
        if let Some(interval) = config.flush_interval {
            if let Some(w) = writer.as_mut() {
                if last_flush.elapsed() >= interval {
                    w.flush_to_disk()?;
                    last_flush = Instant::now();
                }
            }
        }

        // Sleep to maintain poll rate
        sleep_remaining(tick_start, config.poll_interval);
    }
}

/// Sleep for the remaining time in the current tick to maintain target poll rate.
fn sleep_remaining(tick_start: Instant, poll_interval: Duration) {
    let elapsed = tick_start.elapsed();
    if elapsed < poll_interval {
        std::thread::sleep(poll_interval - elapsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::source::{ScriptedStep, ScriptedTelemetrySource};
    use crate::shmem::AccGameStatus;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    };
    use crate::writer::TelemetryFrame;
    use crossbeam_channel::bounded;
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

    #[test]
    fn test_engine_records_fake_source() -> TelemetryResult<()> {
        let tmp = std::env::temp_dir().join(format!("test_engine_{}.acctlm",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));

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
            ScriptedStep::new()
                .with_status(AccGameStatus::Off),
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

        let result = run_recording_loop(config, &mut src, tmp.clone(), None, stop_rx, None)?;

        assert_eq!(result.summary.total_samples, 3);
        assert!(result.output_path == tmp);
        assert!(tmp.exists());

        // Verify file is readable via BinaryTelemetryReader
        let reader = crate::BinaryTelemetryReader::open(&tmp)?;
        assert_eq!(reader.summary().total_samples, 3);

        // Cleanup
        let _ = std::fs::remove_file(&tmp);
        Ok(())
    }

    #[test]
    fn test_engine_stop_signal() -> TelemetryResult<()> {
        let tmp = std::env::temp_dir().join(format!("test_engine_stop_{}.acctlm",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));

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

        let result = run_recording_loop(config, &mut src, tmp.clone(), None, stop_rx, None)?;
        handle.join().unwrap();

        // Should have recorded some frames before stopping
        assert!(result.summary.total_samples > 0);
        assert!(result.summary.total_samples < 100); // stopped early

        // Cleanup
        let _ = std::fs::remove_file(&tmp);
        Ok(())
    }
}
