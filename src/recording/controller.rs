//! Recording controller — public API for starting/stopping ACC telemetry recordings.
//!
//! `RecordingController` wraps the shared `run_recording_loop` engine,
//! manages channels, and exposes a thread-safe lifecycle.

use crate::error::{TelemetryError, TelemetryResult};
use crate::recording::engine::{run_recording_loop, LapCompletedCallback, RecordingEngineConfig};
use crate::recording::source::TelemetrySource;
use crate::recording::status::{RecordingStatus, StatusSender, StopReason};
use crate::recording::outcome::{append_lap_index, format_recording_datetime, parse_acctlm_file, RecordingOutcome};
use crate::recording::request::RecordingRequest;
use crate::recording::file_naming;
use crate::dashboard::service::DashboardCommand;
use crate::distributor::TelemetryDistributor;
use crate::item_key::ItemKey;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime};

/// Sending end of the outcome channel (Controller → m1).
pub type OutcomeSender = Sender<RecordingOutcome>;
/// Receiving end of the outcome channel (m1 side).
pub type OutcomeReceiver = Receiver<RecordingOutcome>;

/// Create a bounded outcome channel pair.
pub fn outcome_channel() -> (OutcomeSender, OutcomeReceiver) {
    bounded(1)
}

/// Manages the lifecycle of a single ACC telemetry recording session.
///
/// Created via `RecordingController::start()`. The recording runs in a
/// background thread. Call `stop()` or drop the controller to end recording.
///
/// A controller instance is single-use — create a new one for each recording.
pub struct RecordingController {
    /// Internal stop signal.
    stop_tx: Sender<()>,
    /// Dashboard command channel (for dynamic subscription changes).
    dash_cmd_tx: Option<Sender<DashboardCommand>>,
    /// Recording thread handle.
    handle: Option<JoinHandle<()>>,
    /// Whether stop() has been called (prevents double-stop in Drop).
    stopped: Arc<Mutex<bool>>,
}

impl RecordingController {
    /// Start a new recording session.
    ///
    /// Spawns a background thread that runs the shared recording engine.
    /// Returns immediately with a `RecordingController` handle.
    ///
    /// # Parameters
    /// - `request`: Recording parameters (validated).
    /// - `source`: Telemetry data source (real ACC or fake for testing).
    /// - `status_tx`: Channel for status updates → m1.
    /// - `outcome_tx`: Channel for final outcome → m1.
    /// - `dashboard_tx`: Optional dashboard data channel → m1.
    /// - `lap_completed`: Optional callback invoked when a lap completes.
    pub fn start<S: TelemetrySource + 'static>(
        request: RecordingRequest,
        mut source: S,
        status_tx: StatusSender,
        outcome_tx: OutcomeSender,
        dashboard_tx: Option<Sender<HashMap<String, f64>>>,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<Self> {
        request.validate()?;

        // Read session info NOW so we can validate the output path
        let session = source.session_info()?;
        let output_path = file_naming::build_output_path(
            &request.output_dir,
            &session.track_name,
            &session.car_model,
        )?;

        // Check for collision — public API must not overwrite
        if output_path.exists() {
            return Err(TelemetryError::InvalidArgument(format!(
                "output file already exists: {}",
                output_path.display()
            )));
        }

        let (stop_tx, stop_rx) = bounded::<()>(1);
        let (dash_cmd_tx, dash_cmd_rx) = bounded::<DashboardCommand>(16);
        let stopped = Arc::new(Mutex::new(false));
        let session_track = session.track_name.clone();
        let session_car = session.car_model.clone();

        // Spawn recording thread
        let handle = std::thread::Builder::new()
            .name("recording-engine".into())
            .spawn({
                let status_tx = status_tx.clone();
                let _ = status_tx.send(RecordingStatus::Connected);

                let poll_hz = request.poll_hz;
                let status_interval = request.status_interval;
                let enable_dashboard = request.enable_dashboard;
                let dashboard_items = request.dashboard_items.clone();
                let dashboard_tx = dashboard_tx.clone();

                move || {
                    let _ = status_tx.send(RecordingStatus::RecordingStarted);

                    let engine_config = RecordingEngineConfig {
                        poll_hz,
                        poll_interval: std::time::Duration::from_secs_f64(1.0 / poll_hz.max(1.0)),
                        chunk_rows: 256,
                        status_interval,
                        flush_interval: Some(std::time::Duration::from_secs(2)),
                    };

                    // Set up dashboard if enabled
                    let (distributor, _dash_handle) = if enable_dashboard {
                        let reg = crate::compute::ComputeRegistry::new();
                        // Raw items are auto-available via TelemetryFrame::raw_field_value()
                        // — no explicit registration needed

                        // Subscribe requested items via DashboardService
                        let (_dash_tx, dash_svc) = match &dashboard_tx {
                            Some(tx) => {
                                use crate::dashboard::sink::ChannelSink;
                                let sink = ChannelSink::new(tx.clone());
                                let svc = crate::dashboard::service::DashboardService::new(reg, Box::new(sink));
                                (tx.clone(), svc)
                            }
                            None => {
                                // No dashboard channel — create a dummy sink
                                let (tx, _rx) = bounded::<HashMap<String, f64>>(1);
                                use crate::dashboard::sink::ChannelSink;
                                let sink = ChannelSink::new(tx.clone());
                                let svc = crate::dashboard::service::DashboardService::new(reg, Box::new(sink));
                                (tx, svc)
                            }
                        };

                        let mut dist = TelemetryDistributor::new(1);
                        let dash_rx = dist.add_consumer();

                        // Subscribe to requested dashboard items
                        let mut subscribed_svc = dash_svc;
                        for item in &dashboard_items {
                            let interval = item.interval;
                            let key = match ItemKey::parse(&item.item_name) {
                                Some(k) => k,
                                None => {
                                    let _ = status_tx.send(RecordingStatus::Error {
                                        message: format!("invalid item key: '{}'", item.item_name),
                                        kind: crate::recording::status::RecordingErrorKind::Unknown,
                                    });
                                    continue;
                                }
                            };
                            if let Err(e) = subscribed_svc.subscribe(
                                key,
                                interval,
                                item.reference_source.clone(),
                            ) {
                                let _ = status_tx.send(RecordingStatus::Error {
                                    message: format!("dashboard subscribe failed for '{}': {}", item.item_name, e),
                                    kind: crate::recording::status::RecordingErrorKind::Unknown,
                                });
                            }
                        }

                        let handle = crate::dashboard::spawn_dashboard(subscribed_svc, dash_rx, dash_cmd_rx);
                        (Some(dist), Some(handle))
                    } else {
                        (None, None)
                    };

                    let recording_start = Instant::now();
                    let recording_start_wall = SystemTime::now();
                    let (recording_date, recording_time) =
                        format_recording_datetime(recording_start_wall);

                    match run_recording_loop(
                        engine_config,
                        &mut source,
                        output_path.clone(),
                        distributor.as_ref(),
                        stop_rx,
                        lap_completed,
                    ) {
                        Ok(result) => {
                            let mut summary = result.summary;
                            summary.duration = recording_start.elapsed();

                            // Build and append lap index block to the file
                            let _ = append_lap_index(&output_path);

                            // Parse the just-recorded file for full lap data
                            let outcome = match parse_acctlm_file(&output_path) {
                                Ok(o) => o,
                                Err(e) => {
                                    let _ = status_tx.send(RecordingStatus::Error {
                                        message: format!(
                                            "recording succeeded but failed to parse outcome: {e}"
                                        ),
                                        kind: crate::recording::status::RecordingErrorKind::Unknown,
                                    });
                                    // Fallback: return minimal outcome without lap data
                                    RecordingOutcome {
                                        track_name: session_track,
                                        car_model: session_car,
                                        session_type: "UNKNOWN".into(),
                                        session_type_raw: 0,
                                        file_path: output_path,
                                        file_size_bytes: summary.total_bytes,
                                        total_samples: summary.total_samples,
                                        duration: summary.duration,
                                        recording_date,
                                        recording_time,
                                        laps: vec![],
                                    }
                                }
                            };
                            let _ = status_tx.send(RecordingStatus::Stopping {
                                reason: StopReason::SessionEnd,
                            });
                            let _ = outcome_tx.send(outcome);
                        }
                        Err(e) => {
                            let _ = status_tx.send(RecordingStatus::Error {
                                message: format!("{}", e),
                                kind: crate::recording::status::RecordingErrorKind::Unknown,
                            });
                        }
                    }
                }
            })?;

        // Send initial status
        let _ = status_tx.send(RecordingStatus::Started { thread_id: 0 });

        Ok(Self {
            stop_tx,
            dash_cmd_tx: Some(dash_cmd_tx),
            handle: Some(handle),
            stopped,
        })
    }

    /// 动态添加 dashboard 订阅项
    ///
    /// 可在录制过程中任意时刻调用。如果 dashboard 未启用，命令静默丢弃。
    pub fn add_dashboard_item(&self, item: crate::recording::dashboard::DashboardItemSubscription) {
        if let Some(ref tx) = self.dash_cmd_tx {
            let key = match ItemKey::parse(&item.item_name) {
                Some(k) => k,
                None => return,
            };
            let _ = tx.try_send(DashboardCommand::Subscribe {
                item_key: key,
                interval: item.interval,
                reference_source: item.reference_source,
            });
        }
    }

    /// 动态移除 dashboard 订阅项
    pub fn remove_dashboard_item(&self, key: &ItemKey) {
        if let Some(ref tx) = self.dash_cmd_tx {
            let _ = tx.try_send(DashboardCommand::Unsubscribe(key.clone()));
        }
    }

    /// 原子替换全部 dashboard 订阅
    pub fn replace_dashboard_items(&self, items: &[crate::recording::dashboard::DashboardItemSubscription]) {
        if let Some(ref tx) = self.dash_cmd_tx {
            let mapped: Vec<_> = items.iter().filter_map(|item| {
                let key = ItemKey::parse(&item.item_name)?;
                Some((key, item.interval, item.reference_source.clone()))
            }).collect();
            let _ = tx.try_send(DashboardCommand::ReplaceAll { items: mapped });
        }
    }

    /// Request the recording to stop.
    ///
    /// Sends a stop signal to the recording thread and waits for
    /// graceful shutdown via `join()`.
    pub fn stop(&mut self) {
        let mut guard = match self.stopped.lock() {
            Ok(g) => g,
            Err(_) => return, // mutex poisoned
        };
        if *guard {
            return;
        }
        *guard = true;
        drop(guard);

        let _ = self.stop_tx.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for RecordingController {
    fn drop(&mut self) {
        let guard = match self.stopped.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if !*guard {
            drop(guard);
            let _ = self.stop_tx.send(());
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::source::{ScriptedStep, ScriptedTelemetrySource};
    use crate::recording::status::status_channel;
    use crate::shmem::AccGameStatus;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    };
    use crate::writer::TelemetryFrame;
    use std::time::Duration;

    fn unique_dir() -> std::path::PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("recording_test_{}", ts));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_frame(seed: u64, speed: f32) -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: seed,
            timestamp_ns: seed * 8_333_333,
            controls: ControlSample {
                sample_tick: seed,
                timestamp_ns: seed * 8_333_333,
                physics_packet_id: seed as i32,
                speed_kmh: speed,
                ..ControlSample::default()
            },
            motion: MotionSample::default(),
            tyres: TyreSample::default(),
            powertrain: PowertrainSample::default(),
            session: SessionSample { sample_tick: seed, timestamp_ns: seed * 8_333_333, status: 2, ..SessionSample::default() },
            timing: TimingSample::default(),
            car_state: CarStateSample::default(),
            environment: EnvironmentSample::default(),
            other_cars: OtherCarsSample::default(),
        }
    }

    fn valid_request(dir: &std::path::Path) -> RecordingRequest {
        RecordingRequest {
            poll_hz: 60.0,
            output_dir: dir.to_path_buf(),
            status_interval: Duration::from_secs(1),
            enable_dashboard: false,
            dashboard_items: vec![],
        }
    }

    #[test]
    fn test_start_validates_request() {
        let dir = unique_dir();
        let mut req = valid_request(&dir);
        req.poll_hz = 10.0;
        let (status_tx, _) = status_channel(8);
        let (outcome_tx, _) = outcome_channel();

        let steps = vec![ScriptedStep::new().with_status(AccGameStatus::Live)];
        let src = ScriptedTelemetrySource::new(steps);
        assert!(RecordingController::start(req, src, status_tx, outcome_tx, None, None).is_err());
    }

    #[test]
    fn test_start_with_fake_source() -> TelemetryResult<()> {
        let dir = unique_dir();
        let steps = vec![
            ScriptedStep::new().with_status(AccGameStatus::Live).with_frame(make_frame(0, 100.0)),
            ScriptedStep::new().with_status(AccGameStatus::Live).with_frame(make_frame(1, 150.0)),
            ScriptedStep::new().with_status(AccGameStatus::Off),
        ];
        let src = ScriptedTelemetrySource::new(steps);
        let req = valid_request(&dir);
        let (status_tx, _) = status_channel(16);
        let (outcome_tx, outcome_rx) = outcome_channel();

        let _ctrl = RecordingController::start(req, src, status_tx, outcome_tx, None, None)?;

        let outcome = outcome_rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| TelemetryError::InvalidArgument("timeout".into()))?;
        assert_eq!(outcome.total_samples, 2);
        assert!(outcome.file_path.exists());
        let _ = std::fs::remove_file(&outcome.file_path);
        Ok(())
    }

    #[test]
    fn test_stop_recording() -> TelemetryResult<()> {
        let dir = unique_dir();
        let steps: Vec<ScriptedStep> = (0..200)
            .map(|i| ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(i, 100.0 + i as f32)))
            .collect();
        let src = ScriptedTelemetrySource::new(steps);
        let req = valid_request(&dir);
        let (status_tx, _) = status_channel(16);
        let (outcome_tx, outcome_rx) = outcome_channel();

        let mut ctrl = RecordingController::start(req, src, status_tx, outcome_tx, None, None)?;
        std::thread::sleep(Duration::from_millis(100));
        ctrl.stop();

        let outcome = outcome_rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| TelemetryError::InvalidArgument("timeout".into()))?;
        assert!(outcome.total_samples > 0);
        assert!(outcome.file_path.exists());
        let _ = std::fs::remove_file(&outcome.file_path);
        Ok(())
    }

    #[test]
    fn test_drop_cleanup() -> TelemetryResult<()> {
        let dir = unique_dir();
        let steps: Vec<ScriptedStep> = (0..200)
            .map(|i| ScriptedStep::new()
                .with_status(AccGameStatus::Live)
                .with_frame(make_frame(i, 100.0 + i as f32)))
            .collect();
        let src = ScriptedTelemetrySource::new(steps);
        let req = valid_request(&dir);
        let (status_tx, _) = status_channel(16);
        let (outcome_tx, outcome_rx) = outcome_channel();

        {
            let _ctrl = RecordingController::start(req, src, status_tx, outcome_tx, None, None)?;
            std::thread::sleep(Duration::from_millis(50)); // let thread start
        }

        let outcome = outcome_rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| TelemetryError::InvalidArgument("timeout".into()))?;
        assert!(outcome.file_path.exists());
        let _ = std::fs::remove_file(&outcome.file_path);
        Ok(())
    }
}
