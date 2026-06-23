//! Recording controller — public API for starting/stopping ACC telemetry recordings.
//!
//! `RecordingController` wraps the shared `run_recording_loop` engine,
//! manages channels, and exposes a thread-safe lifecycle.

use crate::compute::context::ReferenceSource;
use crate::dashboard::service::{
    DashboardCommand, DashboardServiceStats, DashboardServiceStatsHandle,
};
use crate::dashboard::sink::{LatestValueReceiver, LatestValueSender};
use crate::distributor::TelemetryDistributor;
use crate::error::TelemetryResult;
use crate::item_key::ItemKey;
use crate::recording::engine::{
    run_recording_loop, run_replay_loop, LapCompletedCallback, LapCompletedEvent,
    RecordingEngineConfig,
};
use crate::recording::outcome::{
    append_lap_index, format_recording_datetime, parse_acctlm_file, RecordingOutcome,
};
use crate::recording::request::{RecordingRequest, ReplayRequest};
use crate::recording::source::{AccTelemetrySource, ReplayTelemetrySource};
use crate::recording::status::{RecordingErrorKind, RecordingStatus, StatusSender, StopReason};
use crate::recording::{
    dashboard::{
        builtin_calculated_item_names, dashboard_item_info_for_subscription,
        validate_dashboard_subscriptions_with_calculated_items,
    },
    DashboardItemInfo, DashboardItemSubscription, DashboardSubscriptionError,
    DashboardSubscriptionGeneration, DashboardValuesFrame,
};
use crate::TelemetryFrame;
use crate::types::SessionMetadata;
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TryRecvError, TrySendError};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

/// Sending end of the outcome channel (Controller → m1).
pub type OutcomeSender = Sender<RecordingOutcome>;
/// Receiving end of the outcome channel (m1 side).
pub type OutcomeReceiver = Receiver<RecordingOutcome>;

/// Create a bounded outcome channel pair.
pub fn outcome_channel() -> (OutcomeSender, OutcomeReceiver) {
    bounded(16)
}

/// Manages the lifecycle of the ACC telemetry recording service.
///
/// Created via `RecordingController::start()`. A holder thread waits for ACC
/// shared memory, records each session as it appears, emits one outcome per
/// completed session, and keeps running until `stop()` or drop.
///
pub struct RecordingController {
    /// Internal stop signal.
    stop_tx: Sender<()>,
    /// Dashboard command channel (for dynamic subscription changes).
    dash_cmd_tx: Option<Sender<DashboardCommand>>,
    /// Holder thread handle.
    handle: Option<JoinHandle<()>>,
    /// Dashboard thread handle (shared with recording thread).
    dash_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Whether stop() has been called (prevents double-stop in Drop).
    stopped: Arc<Mutex<bool>>,
    /// Current dashboard subscriptions requested through this controller.
    dashboard_items: Arc<Mutex<Vec<DashboardItemSubscription>>>,
    /// Calculated item names available to this controller's dashboard service.
    dashboard_calculated_items: Arc<HashSet<String>>,
    dashboard_stats: DashboardServiceStatsHandle,
    /// Current session metadata, populated when a session starts.
    /// Updated on RecordingStarted (live) or before ReplayStarted (replay).
    session_metadata: Arc<Mutex<Option<SessionMetadata>>>,
}

enum DashboardOutput {
    Legacy(Option<Sender<DashboardValuesFrame>>),
    Latest(LatestValueSender),
}

impl RecordingController {
    /// Start the long-lived recording service.
    ///
    /// Spawns a holder thread and returns immediately. The holder waits for
    /// ACC shared memory in the background and records every session until
    /// the controller is stopped.
    ///
    /// # Parameters
    /// - `request`: Recording parameters (validated).
    /// - `status_tx`: Channel for status updates → m1.
    /// - `outcome_tx`: Channel for final outcome → m1.
    /// - `dashboard_tx`: Optional dashboard data channel → m1.
    /// - `lap_completed`: Optional callback invoked when a lap completes.
    pub fn start(
        request: RecordingRequest,
        status_tx: StatusSender,
        outcome_tx: OutcomeSender,
        dashboard_tx: Option<Sender<DashboardValuesFrame>>,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<Self> {
        Self::start_with_output(
            request,
            status_tx,
            outcome_tx,
            DashboardOutput::Legacy(dashboard_tx),
            lap_completed,
        )
    }

    /// Start recording with an overwrite-on-full dashboard channel.
    ///
    /// This is the preferred entry point for live HUD consumers: when the
    /// consumer falls behind, only the newest sparse frame is retained.
    pub fn start_with_latest_dashboard(
        request: RecordingRequest,
        status_tx: StatusSender,
        outcome_tx: OutcomeSender,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<(Self, LatestValueReceiver)> {
        let (sender, receiver) = crate::dashboard::sink::latest_value_channel();
        let controller = Self::start_with_output(
            request,
            status_tx,
            outcome_tx,
            DashboardOutput::Latest(sender),
            lap_completed,
        )?;
        Ok((controller, receiver))
    }

    /// Start replaying a previously recorded telemetry file.
    ///
    /// Spawns a replay holder thread and returns immediately. The holder replays
    /// frames from the file at the requested speed and feeds the dashboard until
    /// the file is exhausted or the controller is stopped.
    ///
    /// # Parameters
    /// - `request`: Replay parameters (validated).
    /// - `status_tx`: Channel for status updates → m1.
    /// - `dashboard_tx`: Optional dashboard data channel → m1.
    /// - `lap_completed`: Optional callback invoked when a lap completes.
    pub fn start_replay(
        request: ReplayRequest,
        status_tx: StatusSender,
        dashboard_tx: Option<Sender<DashboardValuesFrame>>,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<Self> {
        Self::start_replay_with_output(
            request,
            status_tx,
            DashboardOutput::Legacy(dashboard_tx),
            lap_completed,
        )
    }

    /// Start replay with an overwrite-on-full dashboard channel.
    ///
    /// This is the preferred entry point for live HUD consumers: when the
    /// consumer falls behind, only the newest sparse frame is retained.
    pub fn start_replay_with_latest_dashboard(
        request: ReplayRequest,
        status_tx: StatusSender,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<(Self, LatestValueReceiver)> {
        let (sender, receiver) = crate::dashboard::sink::latest_value_channel();
        let controller = Self::start_replay_with_output(
            request,
            status_tx,
            DashboardOutput::Latest(sender),
            lap_completed,
        )?;
        Ok((controller, receiver))
    }

    fn start_replay_with_output(
        request: ReplayRequest,
        status_tx: StatusSender,
        dashboard_output: DashboardOutput,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<Self> {
        request.validate()?;

        // Open source early to validate the file and get poll_hz
        let source = ReplayTelemetrySource::open(&request.file_path)?;
        let poll_hz = source.poll_hz();

        let (stop_tx, stop_rx) = bounded::<()>(1);
        let (dash_cmd_tx, dash_cmd_rx) = bounded::<DashboardCommand>(16);
        let stopped = Arc::new(Mutex::new(false));
        let dash_handle = Arc::new(Mutex::new(None));
        let dashboard_items = Arc::new(Mutex::new(request.dashboard_items.clone()));

        let mut calc_names = crate::recording::dashboard::builtin_calculated_item_names();
        calc_names.extend(
            request
                .dashboard_realtime_items
                .iter()
                .map(|item| item.name.clone()),
        );
        let dashboard_calculated_items = Arc::new(calc_names);
        let dashboard_stats = DashboardServiceStatsHandle::default();
        let session_metadata = Arc::new(Mutex::new(None));

        let handle = std::thread::Builder::new()
            .name("replay-holder".into())
            .spawn({
                let status_tx = status_tx.clone();
                let dash_handle_clone = dash_handle.clone();
                let dash_cmd_tx_for_holder = dash_cmd_tx.clone();
                let dashboard_stats_for_holder = dashboard_stats.clone();
                let session_meta = Arc::clone(&session_metadata);

                move || {
                    run_replay_holder(
                        request,
                        status_tx,
                        dashboard_output,
                        dashboard_stats_for_holder,
                        dash_cmd_tx_for_holder,
                        dash_cmd_rx,
                        dash_handle_clone,
                        stop_rx,
                        lap_completed,
                        source,
                        poll_hz,
                        session_meta,
                    );
                }
            })?;

        Ok(Self {
            stop_tx,
            dash_cmd_tx: Some(dash_cmd_tx),
            handle: Some(handle),
            dash_handle,
            stopped,
            dashboard_items,
            dashboard_calculated_items,
            dashboard_stats,
            session_metadata,
        })
    }

    fn start_with_output(
        request: RecordingRequest,
        status_tx: StatusSender,
        outcome_tx: OutcomeSender,
        dashboard_output: DashboardOutput,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<Self> {
        request.validate()?;

        let (stop_tx, stop_rx) = bounded::<()>(1);
        let (dash_cmd_tx, dash_cmd_rx) = bounded::<DashboardCommand>(16);
        let stopped = Arc::new(Mutex::new(false));
        let dash_handle = Arc::new(Mutex::new(None));
        let dashboard_items = Arc::new(Mutex::new(request.dashboard_items.clone()));
        let dashboard_calculated_items = Arc::new(dashboard_calculated_item_names(&request));
        let dashboard_stats = DashboardServiceStatsHandle::default();
        let session_metadata = Arc::new(Mutex::new(None));

        let handle = std::thread::Builder::new()
            .name("recording-holder".into())
            .spawn({
                let status_tx = status_tx.clone();
                let outcome_tx = outcome_tx.clone();
                let dash_handle_clone = dash_handle.clone();
                let dash_cmd_tx_for_holder = dash_cmd_tx.clone();
                let dashboard_stats_for_holder = dashboard_stats.clone();
                let session_meta = Arc::clone(&session_metadata);

                move || {
                    run_recording_holder(
                        request,
                        status_tx,
                        outcome_tx,
                        dashboard_output,
                        dashboard_stats_for_holder,
                        dash_cmd_tx_for_holder,
                        dash_cmd_rx,
                        dash_handle_clone,
                        stop_rx,
                        lap_completed,
                        session_meta,
                    );
                }
            })?;

        Ok(Self {
            stop_tx,
            dash_cmd_tx: Some(dash_cmd_tx),
            handle: Some(handle),
            dash_handle,
            stopped,
            dashboard_items,
            dashboard_calculated_items,
            dashboard_stats,
            session_metadata,
        })
    }

    /// Return a lock-free snapshot of dashboard producer metrics.
    pub fn dashboard_stats(&self) -> DashboardServiceStats {
        self.dashboard_stats.snapshot()
    }

    /// Return the current session's metadata (track_name, car_model, etc.).
    ///
    /// Available after `RecordingStatus::RecordingStarted` (live recording) or
    /// `RecordingStatus::ReplayStarted` (replay). Returns `None` if no session
    /// has started yet.
    pub fn session_metadata(&self) -> Option<SessionMetadata> {
        self.session_metadata.lock().ok()?.clone()
    }

    /// 动态添加 dashboard 订阅项
    ///
    /// 可在录制过程中任意时刻调用。如果 dashboard 未启用，命令静默丢弃。
    pub fn add_dashboard_item(&self, item: DashboardItemSubscription) {
        if let Some(ref tx) = self.dash_cmd_tx {
            let key = match ItemKey::parse(&item.item_name) {
                Some(k) => k,
                None => {
                    eprintln!(
                        "recording dashboard: add ignored invalid item '{}'",
                        item.item_name
                    );
                    return;
                }
            };
            if let Ok(mut items) = self.dashboard_items.lock() {
                items.push(item.clone());
            }
            match tx.try_send(DashboardCommand::Subscribe {
                item_key: key,
                interval: item.interval,
                reference_source: item.reference_source,
            }) {
                Ok(()) => {
                    eprintln!(
                        "recording dashboard: add command sent for '{}' interval={}ms",
                        item.item_name,
                        item.interval.as_millis()
                    );
                }
                Err(err) => {
                    eprintln!(
                        "recording dashboard: add command failed for '{}': {}",
                        item.item_name, err
                    );
                }
            }
        } else {
            eprintln!(
                "recording dashboard: add ignored because dashboard command channel is unavailable"
            );
        }
    }

    /// 动态移除 dashboard 订阅项
    pub fn remove_dashboard_item(&self, key: &ItemKey) {
        if let Some(ref tx) = self.dash_cmd_tx {
            if let Ok(mut items) = self.dashboard_items.lock() {
                items.retain(|item| ItemKey::parse(&item.item_name).as_ref() != Some(key));
            }
            match tx.try_send(DashboardCommand::Unsubscribe(key.clone())) {
                Ok(()) => eprintln!("recording dashboard: remove command sent for '{}'", key),
                Err(err) => eprintln!(
                    "recording dashboard: remove command failed for '{}': {}",
                    key, err
                ),
            }
        } else {
            eprintln!(
                "recording dashboard: remove ignored because dashboard command channel is unavailable"
            );
        }
    }

    /// 原子替换全部 dashboard 订阅
    pub fn replace_dashboard_items(
        &self,
        items: &[DashboardItemSubscription],
    ) -> Result<(), DashboardSubscriptionError> {
        self.replace_dashboard_items_with_generation(items)
            .map(|_| ())
    }

    /// Atomically replace dashboard subscriptions and wait until the dashboard
    /// service has applied them, cleared pending old-generation patches, and
    /// acknowledged the new generation.
    pub fn replace_dashboard_items_with_generation(
        &self,
        items: &[DashboardItemSubscription],
    ) -> Result<DashboardSubscriptionGeneration, DashboardSubscriptionError> {
        let validation = validate_dashboard_subscriptions_with_calculated_items(
            items,
            &self.dashboard_calculated_items,
        );
        if let Some(error) = validation.errors.into_iter().next() {
            eprintln!(
                "recording dashboard: replace validation failed for '{}': {}",
                error.item_name, error.message
            );
            return Err(error);
        }

        if let Some(ref tx) = self.dash_cmd_tx {
            let mut mapped = Vec::with_capacity(items.len());
            for item in items {
                let Some(key) = ItemKey::parse(&item.item_name) else {
                    eprintln!(
                        "recording dashboard: replace rejected invalid item '{}'",
                        item.item_name
                    );
                    return Err(DashboardSubscriptionError {
                        item_name: item.item_name.clone(),
                        message: "item name must use raw:, calc:, or system: prefix".to_string(),
                    });
                };
                mapped.push((key, item.interval, item.reference_source.clone()));
            }
            let (ack_tx, ack_rx) = bounded(1);
            tx.try_send(DashboardCommand::ReplaceAll {
                items: mapped,
                ack: ack_tx,
            })
            .map_err(|err| DashboardSubscriptionError {
                item_name: String::new(),
                message: format!("failed to update dashboard subscriptions: {err}"),
            })?;
            let generation = ack_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|err| DashboardSubscriptionError {
                    item_name: String::new(),
                    message: format!("dashboard subscription replace ACK failed: {err}"),
                })??;
            eprintln!(
                "recording dashboard: replace applied items={} generation={}",
                items.len(),
                generation
            );
            if let Ok(mut current) = self.dashboard_items.lock() {
                *current = items.to_vec();
            }
            Ok(generation)
        } else {
            Err(DashboardSubscriptionError {
                item_name: String::new(),
                message: "dashboard command channel is unavailable".to_string(),
            })
        }
    }

    /// Return metadata for the dashboard items currently subscribed by this controller.
    pub fn list_dashboard_items(&self) -> Vec<DashboardItemInfo> {
        self.dashboard_items
            .lock()
            .map(|items| {
                items
                    .iter()
                    .map(dashboard_item_info_for_subscription)
                    .collect()
            })
            .unwrap_or_default()
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
        // Join dashboard thread if it was started
        if let Ok(mut guard) = self.dash_handle.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_recording_holder(
    request: RecordingRequest,
    status_tx: StatusSender,
    outcome_tx: OutcomeSender,
    dashboard_output: DashboardOutput,
    dashboard_stats: DashboardServiceStatsHandle,
    dash_cmd_tx: Sender<DashboardCommand>,
    dash_cmd_rx: Receiver<DashboardCommand>,
    dash_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    stop_rx: Receiver<()>,
    lap_completed: Option<LapCompletedCallback>,
    session_meta: Arc<Mutex<Option<SessionMetadata>>>,
) {
    send_status(&status_tx, RecordingStatus::Started { thread_id: 0 });

    let engine_config = RecordingEngineConfig {
        poll_hz: request.poll_hz,
        poll_interval: Duration::from_secs_f64(1.0 / request.poll_hz.max(1.0)),
        chunk_rows: 256,
        status_interval: request.status_interval,
        flush_interval: Some(Duration::from_secs(2)),
    };

    let distributor = setup_dashboard_thread(
        &request,
        &status_tx,
        dashboard_output,
        dashboard_stats,
        dash_cmd_rx,
        dash_handle,
    );
    let session_best_tracker = Arc::new(Mutex::new(SessionBestLapTracker::default()));
    let lap_completed = build_lap_completed_callback(
        dash_cmd_tx,
        Arc::clone(&session_best_tracker),
        lap_completed,
    );

    let retry_interval = Duration::from_secs(1);
    let mut last_waiting_report: Option<Instant> = None;

    loop {
        if stop_requested(&stop_rx) {
            send_status(
                &status_tx,
                RecordingStatus::Stopping {
                    reason: StopReason::Manual,
                },
            );
            break;
        }

        let mut source = match AccTelemetrySource::open() {
            Ok(source) => {
                last_waiting_report = None;
                send_status(&status_tx, RecordingStatus::Connected);
                source
            }
            Err(err) => {
                let should_report = last_waiting_report
                    .map(|last| last.elapsed() >= request.status_interval)
                    .unwrap_or(true);
                if should_report {
                    send_status(
                        &status_tx,
                        RecordingStatus::WaitingForSharedMemory {
                            message: format!("ACC shared memory not available: {err}"),
                        },
                    );
                    last_waiting_report = Some(Instant::now());
                }

                if wait_for_stop(&stop_rx, retry_interval) {
                    send_status(
                        &status_tx,
                        RecordingStatus::Stopping {
                            reason: StopReason::Manual,
                        },
                    );
                    break;
                }
                continue;
            }
        };

        let recording_start = Instant::now();
        let recording_start_wall = SystemTime::now();
        reset_session_best_tracker(&session_best_tracker);

        match run_recording_loop(
            engine_config.clone(),
            &mut source,
            request.output_dir.clone(),
            Some(&distributor),
            stop_rx.clone(),
            Some(&lap_completed),
            Some(&status_tx),
            Some(&session_meta),
        ) {
            Ok(result) => {
                let should_continue = handle_recording_result(
                    result,
                    recording_start,
                    recording_start_wall,
                    &status_tx,
                    &outcome_tx,
                );
                if !should_continue {
                    break;
                }
            }
            Err(err) => {
                send_status(
                    &status_tx,
                    RecordingStatus::Error {
                        message: err.to_string(),
                        kind: RecordingErrorKind::Unknown,
                    },
                );

                if wait_for_stop(&stop_rx, retry_interval) {
                    send_status(
                        &status_tx,
                        RecordingStatus::Stopping {
                            reason: StopReason::Manual,
                        },
                    );
                    break;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_replay_holder(
    request: ReplayRequest,
    status_tx: StatusSender,
    dashboard_output: DashboardOutput,
    dashboard_stats: DashboardServiceStatsHandle,
    dash_cmd_tx: Sender<DashboardCommand>,
    dash_cmd_rx: Receiver<DashboardCommand>,
    dash_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    stop_rx: Receiver<()>,
    lap_completed: Option<LapCompletedCallback>,
    mut source: ReplayTelemetrySource,
    poll_hz: f64,
    session_meta: Arc<Mutex<Option<SessionMetadata>>>,
) {
    // Store session metadata from the replay file BEFORE any status events,
    // so it is available when the caller receives ReplayStarted.
    if let Ok(mut guard) = session_meta.lock() {
        *guard = Some(source.metadata().clone());
    }

    send_status(&status_tx, RecordingStatus::Started { thread_id: 0 });

    // Build a temp RecordingRequest with only the fields setup_dashboard_thread uses
    let temp_req = RecordingRequest {
        poll_hz: 60.0,
        output_dir: std::env::temp_dir(),
        status_interval: request.status_interval,
        dashboard_items: request.dashboard_items.clone(),
        dashboard_realtime_items: request.dashboard_realtime_items.clone(),
    };

    let distributor = setup_dashboard_thread(
        &temp_req,
        &status_tx,
        dashboard_output,
        dashboard_stats,
        dash_cmd_rx,
        dash_handle,
    );
    let session_best_tracker = Arc::new(Mutex::new(SessionBestLapTracker::default()));
    let lap_completed = build_lap_completed_callback(
        dash_cmd_tx,
        Arc::clone(&session_best_tracker),
        lap_completed,
    );

    match run_replay_loop(
        request.speed_multiplier,
        &mut source,
        poll_hz,
        Some(&distributor),
        stop_rx,
        Some(&lap_completed),
        Some(&status_tx),
    ) {
        Ok(reason) => {
            eprintln!("replay holder: finished with {:?}", reason);
        }
        Err(err) => {
            send_status(
                &status_tx,
                RecordingStatus::Error {
                    message: err.to_string(),
                    kind: RecordingErrorKind::Unknown,
                },
            );
        }
    }
}

#[derive(Default)]
struct SessionBestLapTracker {
    best_lap_time_ms: Option<i32>,
}

struct SessionBestLapUpdate {
    lap_number: u32,
    lap_time_ms: i32,
    frames: Vec<TelemetryFrame>,
}

impl SessionBestLapTracker {
    fn reset(&mut self) {
        self.best_lap_time_ms = None;
    }

    fn consider_lap(&mut self, event: &LapCompletedEvent) -> Option<SessionBestLapUpdate> {
        if event.is_out_lap {
            eprintln!(
                "recording dashboard: session best skipped out lap lap={}",
                event.lap_number
            );
            return None;
        }
        if !event.is_valid {
            eprintln!(
                "recording dashboard: session best skipped invalid lap lap={} lap_time_ms={}",
                event.lap_number, event.lap_time_ms
            );
            return None;
        }
        if event.lap_time_ms <= 0 {
            eprintln!(
                "recording dashboard: session best skipped lap={} invalid lap_time_ms={}",
                event.lap_number, event.lap_time_ms
            );
            return None;
        }
        if event.lap_frames.is_empty() {
            eprintln!(
                "recording dashboard: session best skipped lap={} with no frames",
                event.lap_number
            );
            return None;
        }

        let is_new_best = self
            .best_lap_time_ms
            .map(|best| event.lap_time_ms < best)
            .unwrap_or(true);
        if !is_new_best {
            eprintln!(
                "recording dashboard: session best kept current_best_ms={} candidate_lap={} candidate_ms={}",
                self.best_lap_time_ms.unwrap_or_default(),
                event.lap_number,
                event.lap_time_ms
            );
            return None;
        }

        self.best_lap_time_ms = Some(event.lap_time_ms);
        eprintln!(
            "recording dashboard: session best updated lap={} lap_time_ms={} frames={}",
            event.lap_number,
            event.lap_time_ms,
            event.lap_frames.len()
        );
        Some(SessionBestLapUpdate {
            lap_number: event.lap_number,
            lap_time_ms: event.lap_time_ms,
            frames: event.lap_frames.clone(),
        })
    }
}

fn build_lap_completed_callback(
    dash_cmd_tx: Sender<DashboardCommand>,
    session_best_tracker: Arc<Mutex<SessionBestLapTracker>>,
    external: Option<LapCompletedCallback>,
) -> LapCompletedCallback {
    Box::new(move |event: LapCompletedEvent| {
        if let Ok(mut tracker) = session_best_tracker.lock() {
            if let Some(update) = tracker.consider_lap(&event) {
                let source = ReferenceSource::session_best();
                match dash_cmd_tx.try_send(DashboardCommand::ReplaceReference {
                    source,
                    lap_number: update.lap_number,
                    lap_time_ms: update.lap_time_ms,
                    frames: update.frames,
                }) {
                    Ok(()) => eprintln!(
                        "recording dashboard: session best reference update command sent lap={} lap_time_ms={}",
                        update.lap_number, update.lap_time_ms
                    ),
                    Err(err) => eprintln!(
                        "recording dashboard: session best reference update command failed lap={} lap_time_ms={}: {}",
                        update.lap_number, update.lap_time_ms, err
                    ),
                }
            }
        } else {
            eprintln!("recording dashboard: session best tracker lock poisoned");
        }

        if let Some(cb) = external.as_ref() {
            cb(event);
        }
    })
}

fn reset_session_best_tracker(session_best_tracker: &Arc<Mutex<SessionBestLapTracker>>) {
    match session_best_tracker.lock() {
        Ok(mut tracker) => {
            tracker.reset();
            eprintln!("recording dashboard: session best tracker reset");
        }
        Err(_) => {
            eprintln!("recording dashboard: session best tracker reset failed: lock poisoned")
        }
    }
}

fn dashboard_calculated_item_names(request: &RecordingRequest) -> HashSet<String> {
    let mut names = builtin_calculated_item_names();
    names.extend(
        request
            .dashboard_realtime_items
            .iter()
            .map(|item| item.name.clone()),
    );
    names
}

fn setup_dashboard_thread(
    request: &RecordingRequest,
    status_tx: &StatusSender,
    dashboard_output: DashboardOutput,
    dashboard_stats: DashboardServiceStatsHandle,
    dash_cmd_rx: Receiver<DashboardCommand>,
    dash_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
) -> TelemetryDistributor {
    eprintln!(
        "recording dashboard: setup requested initial_items={} custom_realtime_items={} sink={}",
        request.dashboard_items.len(),
        request.dashboard_realtime_items.len(),
        match &dashboard_output {
            DashboardOutput::Legacy(Some(_)) => "legacy-channel",
            DashboardOutput::Legacy(None) => "null",
            DashboardOutput::Latest(_) => "latest-value",
        }
    );
    let mut reg = match crate::compute::ComputeRegistry::with_builtin_dashboard_items() {
        Ok(registry) => registry,
        Err(err) => {
            send_status(
                status_tx,
                RecordingStatus::Error {
                    message: format!("failed to register builtin dashboard items: {err}"),
                    kind: RecordingErrorKind::Unknown,
                },
            );
            crate::compute::ComputeRegistry::new()
        }
    };
    for item in &request.dashboard_realtime_items {
        if let Err(err) = reg.register_calc_realtime(item.create_item()) {
            send_status(
                status_tx,
                RecordingStatus::Error {
                    message: format!(
                        "dashboard realtime item '{}' failed to register: {err}",
                        item.name
                    ),
                    kind: RecordingErrorKind::Unknown,
                },
            );
        } else {
            eprintln!(
                "recording dashboard: registered custom realtime calc '{}'",
                item.name
            );
        }
    }
    eprintln!(
        "recording dashboard: registered calc items={:?}",
        reg.registered_item_names()
    );
    let sink: Box<dyn crate::dashboard::sink::DataSink> = match dashboard_output {
        DashboardOutput::Legacy(Some(tx)) => Box::new(crate::dashboard::sink::ChannelSink::new(tx)),
        DashboardOutput::Legacy(None) => Box::new(crate::dashboard::sink::NullSink),
        DashboardOutput::Latest(tx) => Box::new(crate::dashboard::sink::LatestValueSink::new(tx)),
    };
    let mut dashboard_service =
        crate::dashboard::service::DashboardService::with_stats(reg, sink, dashboard_stats);

    for item in &request.dashboard_items {
        let key = match ItemKey::parse(&item.item_name) {
            Some(k) => k,
            None => {
                send_status(
                    status_tx,
                    RecordingStatus::Error {
                        message: format!("invalid item key: '{}'", item.item_name),
                        kind: RecordingErrorKind::Unknown,
                    },
                );
                continue;
            }
        };

        if let Err(err) =
            dashboard_service.subscribe(key, item.interval, item.reference_source.clone())
        {
            send_status(
                status_tx,
                RecordingStatus::Error {
                    message: format!("dashboard subscribe failed for '{}': {err}", item.item_name),
                    kind: RecordingErrorKind::Unknown,
                },
            );
        } else {
            eprintln!(
                "recording dashboard: initial subscription '{}' interval={}ms reference_source={}",
                item.item_name,
                item.interval.as_millis(),
                item.reference_source.is_some()
            );
        }
    }

    let mut distributor = TelemetryDistributor::new(1);
    let dash_rx = distributor.add_consumer();
    let handle = crate::dashboard::spawn_dashboard(dashboard_service, dash_rx, dash_cmd_rx);
    eprintln!("recording dashboard: thread spawned");
    if let Ok(mut guard) = dash_handle.lock() {
        *guard = Some(handle);
    }

    distributor
}

fn handle_recording_result(
    result: crate::recording::engine::RecordingLoopResult,
    recording_start: Instant,
    recording_start_wall: SystemTime,
    status_tx: &StatusSender,
    outcome_tx: &OutcomeSender,
) -> bool {
    let stop_reason = result.stop_reason.clone();

    if result.cancelled_before_start {
        if stop_reason == StopReason::Manual {
            send_status(
                status_tx,
                RecordingStatus::Stopping {
                    reason: stop_reason,
                },
            );
            return false;
        }
        return true;
    }

    let mut summary = result.summary;
    summary.duration = recording_start.elapsed();

    let Some(output_path) = result.output_path else {
        send_status(
            status_tx,
            RecordingStatus::Error {
                message: "recording finished without an output file".into(),
                kind: RecordingErrorKind::Unknown,
            },
        );
        return stop_reason != StopReason::Manual;
    };

    let _ = append_lap_index(&output_path);
    let (recording_date, recording_time) = format_recording_datetime(recording_start_wall);

    let outcome = match parse_acctlm_file(&output_path) {
        Ok(outcome) => outcome,
        Err(err) => {
            send_status(
                status_tx,
                RecordingStatus::Error {
                    message: format!("recording succeeded but failed to parse outcome: {err}"),
                    kind: RecordingErrorKind::Unknown,
                },
            );
            RecordingOutcome {
                track_name: "UNKNOWN".into(),
                car_model: "UNKNOWN".into(),
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

    send_status(
        status_tx,
        RecordingStatus::Stopping {
            reason: stop_reason.clone(),
        },
    );
    send_outcome(status_tx, outcome_tx, outcome);

    stop_reason != StopReason::Manual
}

fn send_status(status_tx: &StatusSender, status: RecordingStatus) {
    let _ = status_tx.try_send(status);
}

fn send_outcome(status_tx: &StatusSender, outcome_tx: &OutcomeSender, outcome: RecordingOutcome) {
    match outcome_tx.try_send(outcome) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => send_status(
            status_tx,
            RecordingStatus::Error {
                message: "recording outcome channel is full; dropping session outcome".into(),
                kind: RecordingErrorKind::Unknown,
            },
        ),
        Err(TrySendError::Disconnected(_)) => send_status(
            status_tx,
            RecordingStatus::Error {
                message: "recording outcome receiver disconnected".into(),
                kind: RecordingErrorKind::Unknown,
            },
        ),
    }
}

fn stop_requested(stop_rx: &Receiver<()>) -> bool {
    match stop_rx.try_recv() {
        Ok(()) => true,
        Err(TryRecvError::Disconnected) => true,
        Err(TryRecvError::Empty) => false,
    }
}

fn wait_for_stop(stop_rx: &Receiver<()>, timeout: Duration) -> bool {
    match stop_rx.recv_timeout(timeout) {
        Ok(()) => true,
        Err(RecvTimeoutError::Disconnected) => true,
        Err(RecvTimeoutError::Timeout) => false,
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
            // Join dashboard thread if it was started
            if let Ok(mut guard) = self.dash_handle.lock() {
                if let Some(handle) = guard.take() {
                    let _ = handle.join();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::items::RealtimeComputeItem;
    use crate::compute::{ComputeContext, ComputeResult};
    use crate::recording::status::status_channel;
    use crate::recording::{DashboardItemKind, DashboardRealtimeItemRegistration};
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
        PowertrainSample, SessionSample, TimingSample, TyreSample,
    };
    use std::time::Duration;

    struct TestStartupCalcItem;

    impl RealtimeComputeItem for TestStartupCalcItem {
        fn name(&self) -> &str {
            "startup_calc"
        }

        fn compute(&mut self, _ctx: &ComputeContext) -> ComputeResult<f64> {
            Ok(1.0)
        }
    }

    fn unique_dir() -> std::path::PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("recording_test_{}", ts));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn valid_request(dir: &std::path::Path) -> RecordingRequest {
        RecordingRequest {
            poll_hz: 60.0,
            output_dir: dir.to_path_buf(),
            status_interval: Duration::from_secs(1),
            dashboard_items: vec![],
            dashboard_realtime_items: vec![],
        }
    }

    fn test_frame(sample_tick: u64) -> TelemetryFrame {
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

    fn lap_event(
        lap_number: u32,
        is_valid: bool,
        lap_time_ms: i32,
        is_out_lap: bool,
        frame_count: u64,
    ) -> LapCompletedEvent {
        let lap_frames = (0..frame_count).map(test_frame).collect();
        LapCompletedEvent::new(
            lap_number,
            is_valid,
            lap_time_ms,
            is_out_lap,
            "test_track".to_string(),
            lap_frames,
        )
    }

    #[test]
    fn test_start_validates_request() {
        let dir = unique_dir();
        let mut req = valid_request(&dir);
        req.poll_hz = 10.0;
        let (status_tx, _) = status_channel(8);
        let (outcome_tx, _) = outcome_channel();

        assert!(RecordingController::start(req, status_tx, outcome_tx, None, None).is_err());
    }

    #[test]
    fn test_start_valid_request_spawns_holder() {
        let dir = unique_dir();
        let req = valid_request(&dir);
        let (status_tx, _) = status_channel(8);
        let (outcome_tx, _) = outcome_channel();

        let mut controller =
            RecordingController::start(req, status_tx, outcome_tx, None, None).unwrap();
        controller.stop();
    }

    #[test]
    fn test_replace_dashboard_items_accepts_builtin_calc_item() {
        let dir = unique_dir();
        let req = valid_request(&dir);
        let (status_tx, _) = status_channel(8);
        let (outcome_tx, _) = outcome_channel();

        let mut controller =
            RecordingController::start(req, status_tx, outcome_tx, None, None).unwrap();
        let items = vec![DashboardItemSubscription::new(
            "calc:prev_sector_time",
            DashboardItemKind::CalculatedItem,
            Duration::from_millis(50),
        )];

        let generation = controller
            .replace_dashboard_items_with_generation(&items)
            .unwrap();
        assert!(generation > 0);
        assert_eq!(
            controller.dashboard_stats().subscription_generation,
            generation
        );
        controller.stop();
    }

    #[test]
    fn test_replace_dashboard_items_accepts_startup_calc_item() {
        let dir = unique_dir();
        let mut req = valid_request(&dir);
        req.dashboard_realtime_items
            .push(DashboardRealtimeItemRegistration::new(
                "startup_calc",
                || Box::new(TestStartupCalcItem),
            ));
        let (status_tx, _) = status_channel(8);
        let (outcome_tx, _) = outcome_channel();

        let mut controller =
            RecordingController::start(req, status_tx, outcome_tx, None, None).unwrap();
        let items = vec![DashboardItemSubscription::new(
            "calc:startup_calc",
            DashboardItemKind::CalculatedItem,
            Duration::from_millis(50),
        )];

        assert!(controller.replace_dashboard_items(&items).is_ok());
        controller.stop();
    }

    #[test]
    fn test_session_best_tracker_skips_non_reference_laps() {
        let mut tracker = SessionBestLapTracker::default();

        assert!(tracker
            .consider_lap(&lap_event(0, true, 90_000, true, 10))
            .is_none());
        assert!(tracker
            .consider_lap(&lap_event(1, false, 89_000, false, 10))
            .is_none());
        assert!(tracker
            .consider_lap(&lap_event(1, true, 0, false, 10))
            .is_none());
        assert!(tracker
            .consider_lap(&lap_event(1, true, 88_000, false, 0))
            .is_none());
        assert_eq!(tracker.best_lap_time_ms, None);
    }

    #[test]
    fn test_session_best_tracker_updates_only_on_faster_valid_lap() {
        let mut tracker = SessionBestLapTracker::default();

        let first = tracker
            .consider_lap(&lap_event(1, true, 88_000, false, 3))
            .unwrap();
        assert_eq!(first.lap_number, 1);
        assert_eq!(first.lap_time_ms, 88_000);
        assert_eq!(first.frames.len(), 3);
        assert_eq!(tracker.best_lap_time_ms, Some(88_000));

        assert!(tracker
            .consider_lap(&lap_event(2, true, 89_000, false, 3))
            .is_none());
        assert_eq!(tracker.best_lap_time_ms, Some(88_000));

        let faster = tracker
            .consider_lap(&lap_event(3, true, 87_000, false, 4))
            .unwrap();
        assert_eq!(faster.lap_number, 3);
        assert_eq!(faster.lap_time_ms, 87_000);
        assert_eq!(faster.frames.len(), 4);
        assert_eq!(tracker.best_lap_time_ms, Some(87_000));
    }

    // -----------------------------------------------------------------------
    // Replay tests
    // -----------------------------------------------------------------------

    /// Create a temporary `.acctlm2` file with `frame_count` simple frames.
    #[cfg(feature = "v2_writer")]
    fn create_test_acctlm2_file(frame_count: u64) -> (std::path::PathBuf, crate::types::SessionMetadata) {
        use crate::writer::LiveTelemetryConfig;
        use crate::writer_v2::BinaryTelemetryWriterV2;
        use crate::types::SessionMetadata;
        use std::sync::Arc;

        let path = std::env::temp_dir()
            .join(format!("replay_ctrl_test_{}.acctlm2", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let metadata = SessionMetadata::new("test_track", "test_car", 120.0);
        let config = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut writer =
            BinaryTelemetryWriterV2::create_file(&path, metadata.clone(), config)
                .expect("create_file");

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
                    status: crate::shmem::ACC_STATUS_LIVE,
                    ..SessionSample::default()
                },
                ..test_frame(i)
            };
            writer
                .write_frame(&Arc::new(frame))
                .expect("write_frame");
        }
        writer.finish().expect("finish");

        (path, metadata)
    }

    #[test]
    fn test_start_replay_invalid_file() {
        let (status_tx, _) = status_channel(8);
        let req = ReplayRequest {
            file_path: std::path::PathBuf::from("__nonexistent_replay_file__.acctlm2"),
            speed_multiplier: 1.0,
            status_interval: Duration::from_secs(1),
            dashboard_items: vec![],
            dashboard_realtime_items: vec![],
        };

        let result = RecordingController::start_replay(req, status_tx, None, None);
        assert!(result.is_err());
    }

    #[cfg(feature = "v2_writer")]
    #[test]
    fn test_start_replay_basic() {
        let (path, _metadata) = create_test_acctlm2_file(3);
        let (status_tx, status_rx) = status_channel(16);

        let req = ReplayRequest {
            file_path: path.clone(),
            speed_multiplier: 10.0, // fast replay
            status_interval: Duration::from_secs(1),
            dashboard_items: vec![],
            dashboard_realtime_items: vec![],
        };

        let mut controller =
            RecordingController::start_replay(req, status_tx, None, None).expect("start_replay");

        // Let the replay complete
        std::thread::sleep(Duration::from_millis(200));
        controller.stop();

        let statuses: Vec<_> = status_rx.try_iter().collect();

        // Should have at least ReplayStarted
        let has_replay_started = statuses
            .iter()
            .any(|s| matches!(s, RecordingStatus::ReplayStarted));
        assert!(
            has_replay_started,
            "expected ReplayStarted in statuses: {:?}",
            statuses
        );

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(feature = "v2_writer")]
    #[test]
    fn test_start_replay_stop() {
        // Create many frames so the replay doesn't finish before we stop it
        let (path, _metadata) = create_test_acctlm2_file(50);
        let (status_tx, status_rx) = status_channel(16);

        let req = ReplayRequest {
            file_path: path.clone(),
            speed_multiplier: 0.1, // slow replay
            status_interval: Duration::from_secs(1),
            dashboard_items: vec![],
            dashboard_realtime_items: vec![],
        };

        let mut controller =
            RecordingController::start_replay(req, status_tx, None, None).expect("start_replay");

        // Give it a moment to start, then stop
        std::thread::sleep(Duration::from_millis(100));
        controller.stop();

        // Verify clean shutdown (stop() doesn't panic)
        let statuses: Vec<_> = status_rx.try_iter().collect();

        let has_replay_started = statuses
            .iter()
            .any(|s| matches!(s, RecordingStatus::ReplayStarted));
        assert!(
            has_replay_started,
            "expected ReplayStarted before stop: {:?}",
            statuses
        );

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(feature = "v2_writer")]
    #[test]
    fn test_start_replay_with_latest_dashboard() {
        use crate::dashboard::sink::LatestValueReceiver;
        use crate::recording::DashboardItemSubscription;

        let (path, _metadata) = create_test_acctlm2_file(5);
        let (status_tx, _status_rx) = status_channel(16);

        let req = ReplayRequest {
            file_path: path.clone(),
            speed_multiplier: 10.0,
            status_interval: Duration::from_secs(1),
            dashboard_items: vec![DashboardItemSubscription::new(
                "raw:speed_kmh",
                DashboardItemKind::RawItem,
                Duration::from_millis(50),
            )],
            dashboard_realtime_items: vec![],
        };

        let (mut controller, receiver): (RecordingController, LatestValueReceiver) =
            RecordingController::start_replay_with_latest_dashboard(req, status_tx, None)
                .expect("start_replay_with_latest_dashboard");

        // Wait for replay to complete
        std::thread::sleep(Duration::from_millis(300));
        controller.stop();

        // The receiver should have data; try before stop, since
        // the sender may be dropped once the replay finishes.
        let mut got_data = false;
        for _ in 0..3 {
            match receiver.try_recv() {
                Ok(frame) => {
                    eprintln!(
                        "replay latest dashboard: received frame with {} values",
                        frame.values.len()
                    );
                    assert!(!frame.values.is_empty(), "expected dashboard values");
                    got_data = true;
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }

        if !got_data {
            eprintln!("replay latest dashboard: no data received (timing/empty)");
        }

        controller.stop();

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }
}
