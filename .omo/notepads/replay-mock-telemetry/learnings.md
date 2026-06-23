# Replay Controller Implementation - Learnings (Task 5)

## Changes Made

### 1. src/recording/source.rs
- Added `pub(crate) fn poll_hz(&self) -> f64` accessor on ReplayTelemetrySource

### 2. src/recording/controller.rs
**Imports:** Added run_replay_loop, ReplayRequest, ReplayTelemetrySource

**New public methods:**
- start_replay(request, status_tx, dashboard_tx, lap_completed) - wraps start_replay_with_output with Legacy output
- start_replay_with_latest_dashboard(request, status_tx, lap_completed) - creates latest_value_channel, wraps with Latest output

**New private method:**
- start_replay_with_output(request, status_tx, dashboard_output, lap_completed) - validates, opens source, gets poll_hz, spawns replay-holder thread

**New function:**
- run_replay_holder(...) - sends Started, creates temp RecordingRequest for setup_dashboard_thread, calls build_lap_completed_callback, runs run_replay_loop

### 3. src/recording/mod.rs
- Added pub use source::ReplayTelemetrySource

### 4. src/recording/request.rs (bugfix)
- Fixed flaky valid_replay_request() test: removed unnecessary File::open, used PID-unique filename

## Tests Added
- test_start_replay_invalid_file: non-existent file -> Err
- test_start_replay_basic (v2_writer): 3-frame replay, verify ReplayStarted
- test_start_replay_stop (v2_writer): 50-frame slow replay, mid-stream stop -> clean
- test_start_replay_with_latest_dashboard (v2_writer): 5-frame replay + dashboard, verify receiver

## Results
- cargo test replay --features v2_writer: 21 passed, 0 failed
- cargo build: clean
