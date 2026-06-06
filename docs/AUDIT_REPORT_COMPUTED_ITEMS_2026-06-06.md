# Computed Items System Audit Report

## Audit Baseline

- Audit date: 2026-06-06
- Current commit: `fc0b97aac8cb30e8b7ac7569400fb8436e67fb3e`
- Scope: new functionality added after telemetry reader audit fixes, mainly the plan in `.sisyphus/plans/computed-items-system.md`
- Focus areas:
  - `src/compute/*`
  - `src/dashboard/*`
  - `src/distributor.rs`
  - `src/bin/acc-live-telemetry.rs` dashboard/serve integration
  - `tests/compute_tests.rs`
  - `tests/dashboard_tests.rs`

## User Clarification Captured During Audit

The user clarified the expected design for realtime computed items:

- When an upstream caller needs a realtime computed item, the caller must pass all information required by that computation through parameters/context.
- `compute_realtime` should not rely on hidden global state.
- `compute_realtime` should not execute every registered realtime item merely because one subscribed item is due.
- A fix for dashboard subscription scheduling must not reduce or filter the raw ACC shared-memory data recorded by `record` or `record-raw`.

## Impact Assessment: Does Fixing Dashboard Scheduling Affect Recording Completeness?

No, not if the fix is kept inside the compute/dashboard path.

`record` reads a full `TelemetryFrame` from ACC shared memory and writes that frame to `BinaryTelemetryWriter`:

- `src/bin/acc-live-telemetry.rs:201` reads `active_reader.read_telemetry_frame(...)`
- `src/bin/acc-live-telemetry.rs:208` writes `active_writer.write_frame(frame)?`

The dashboard path is a side branch:

- `src/bin/acc-live-telemetry.rs:203` sends a clone to `TelemetryDistributor` only when `--dashboard` is enabled

Therefore, changing `DashboardService`/`ComputeRegistry` so they compute only the requested subscribed item does not change which ACC shared-memory fields are read or written to `.acctlm`.

`record-raw` is even more separate. It writes raw physics and graphics pages directly:

- `src/bin/acc-live-telemetry.rs:702` reads raw physics
- `src/bin/acc-live-telemetry.rs:703` reads raw graphics
- `src/bin/acc-live-telemetry.rs:706` to `src/bin/acc-live-telemetry.rs:709` writes tick, timestamp, physics bytes, graphics bytes

The compute/dashboard modules are not in the `record-raw` path.

Important implementation guardrail: avoid changing the shared-memory reader to collect only fields needed by a computed item. Computed items should consume an already captured full `TelemetryFrame` or explicitly passed reference data. They should not decide what `record` records.

## Findings

### P1: CLI dashboard output is computed but discarded

- Locations:
  - `src/bin/acc-live-telemetry.rs:94`
  - `src/bin/acc-live-telemetry.rs:1224`
  - `src/dashboard/sink.rs:31`
- Symptom:
  - Both `record --dashboard` and `serve` create `(sink_tx, _sink_rx)`.
  - `_sink_rx` is not stored, read, exposed, printed, or forwarded.
  - `ChannelSink::send()` uses `try_send` and ignores errors.
- Impact:
  - DashboardService can receive frames and compute values, but no upstream program can observe the results.
  - `serve` currently has no real data-service output.
- Recommendation:
  - Define the intended output surface first: stdout JSON lines, TCP, WebSocket, HTTP, plugin callback, or an in-process returned receiver.
  - For a minimal CLI-safe fix, use a result consumer thread that reads `sink_rx` and prints newline-delimited JSON or a stable text format.
  - Make send failures observable at least once per sink, instead of swallowing all errors silently.
  - Add an integration test proving that the CLI/dashboard wiring produces a consumable `HashMap<String, f64>`.

### P1: Dynamic realtime item `DeltaToBestLap` cannot run through the registry/dashboard path

- Locations:
  - `src/compute/items.rs:106`
  - `src/compute/registry.rs:68`
  - `src/compute/registry.rs:111`
- Symptom:
  - `DeltaToBestLap::compute()` requires `ctx.reference_lap`.
  - `ComputeRegistry::compute_realtime()` always passes `reference_lap: None`.
  - `ComputeRegistry` has a `reference_cache`, but realtime computation does not use it.
- Impact:
  - The dynamic example item works only in unit tests that manually construct `ComputeContext::with_reference`.
  - It cannot be used by `DashboardService` or `serve` as currently wired.
- Recommendation:
  - Introduce a realtime request/context object, for example:

    ```rust
    pub struct RealtimeComputeRequest<'a> {
        pub item_name: &'a str,
        pub frame: &'a TelemetryFrame,
        pub computed_values: &'a HashMap<String, f64>,
        pub reference_lap: Option<&'a [TelemetryFrame]>,
        pub reference_source: Option<ReferenceSource>,
    }
    ```

  - Add a method such as `compute_realtime_item(request) -> ComputeResult<f64>`.
  - Let dashboard subscriptions optionally carry item-specific context, including `ReferenceSource`.
  - If using `reference_cache`, resolve the reference lap explicitly from the subscription/request before invoking the item.
  - Add an end-to-end dashboard test where `delta_to_best_lap` is subscribed with a reference lap and returns a value.

### P2: Dashboard subscription scheduling currently executes all registered realtime items

- Locations:
  - `src/dashboard/service.rs:83`
  - `src/dashboard/service.rs:95`
  - `src/compute/registry.rs:71`
- Symptom:
  - `DashboardService` collects the names that are due in `items_to_compute`.
  - It then calls `self.registry.compute_realtime(frame)`.
  - `compute_realtime()` iterates over every registered realtime item, then Dashboard filters the resulting map.
- Impact:
  - Unsubscribed items still consume CPU.
  - Unsubscribed stateful items still mutate their internal state.
  - Unsubscribed failing items still emit errors.
  - This violates the plan goal of sparse, on-demand results at per-item frequencies.
- Recording impact:
  - Fixing this does not affect `record` or `record-raw` recording completeness if the fix only changes compute/dashboard execution.
  - The full `TelemetryFrame` should still be read and recorded before/independent of compute item selection.
- Recommendation:
  - Replace all-item execution with item-targeted execution:

    ```rust
    registry.compute_realtime_item(item_name, ctx)
    ```

  - If computed items can depend on previously computed items, represent dependencies explicitly and evaluate only the required dependency closure in registration order.
  - Make all item inputs explicit through a request/context parameter, matching the user clarification above.
  - Add tests:
    - Subscribe only item A; item B must not execute.
    - Subscribe item A at 50 ms and item B at 200 ms; each must execute at its own cadence.
    - A missing reference for `delta_to_best_lap` should fail only when that item is requested.

### P2: `record --dashboard` clones the whole `TelemetryFrame` on the 120 Hz hot path

- Locations:
  - `src/bin/acc-live-telemetry.rs:203`
  - `src/writer.rs:26`
  - `src/types.rs:337`
- Symptom:
  - `record` sends `frame.clone()` to the dashboard branch, then writes the original frame to disk.
  - `TelemetryFrame` contains `OtherCarsSample`, which contains heap-backed vectors:
    - `car_coordinates: Vec<f32>`
    - `car_id: Vec<i32>`
- Impact:
  - Every dashboard-enabled frame can allocate/copy vector data.
  - At 120 Hz this may still be tolerable today, but it violates the intended zero-copy distribution direction and will get worse as consumers/items grow.
- Recommendation:
  - Change the hot path to share one frame allocation:
    - Reader produces `TelemetryFrame`.
    - Wrap it once in `Arc<TelemetryFrame>`.
    - Dashboard clones the `Arc`.
    - Writer encodes by borrowed reference, e.g. `write_frame_ref(&TelemetryFrame)`.
  - Alternatively change `BinaryTelemetryWriter::write_frame` to accept `&TelemetryFrame` and clone only into its chunk buffer when strictly needed.
  - Add a small benchmark or timing test around `record` dashboard distribution overhead.

### P2: Distributor drops the newest frame when a consumer is slow

- Locations:
  - `src/distributor.rs:45`
  - `src/distributor.rs:145`
- Symptom:
  - Documentation implies old frames are dropped for slow consumers.
  - Implementation uses bounded channel `try_send`.
  - When full, crossbeam rejects the new frame, so old queued frames remain.
- Impact:
  - Dashboard can lag behind real telemetry because it processes stale queued frames while new frames are dropped.
- Recommendation:
  - Decide the desired policy:
    - For dashboard/realtime view: prefer latest frame, drop old queued frames.
    - For recorder/audit consumers: prefer lossless or explicit backpressure/error.
  - For latest-frame dashboard behavior, use capacity 1 and replace stale content, or drain pending frames before sending the latest one.
  - Update tests to assert the chosen policy.

### P2: Dashboard thread failures are not observable by the producer

- Locations:
  - `src/dashboard/mod.rs:19`
  - `src/bin/acc-live-telemetry.rs:87`
  - `src/bin/acc-live-telemetry.rs:1229`
  - `src/distributor.rs:48`
- Symptom:
  - CLI stores the dashboard join handle in `_dashboard_handle` and never checks it.
  - A panic in a callback sink or dashboard service kills the dashboard thread.
  - The producer continues calling `try_send`, ignoring errors.
- Impact:
  - Dashboard can silently die while recording continues.
  - This is acceptable only if dashboard is explicitly best-effort and documented as such.
- Recommendation:
  - Periodically check `JoinHandle::is_finished()` in long-running CLI loops.
  - Send dashboard errors to a small error channel and log them from the main loop.
  - Remove or mark disconnected distributor senders to avoid repeated silent failures.

### P2: `DeltaToBestLap` search algorithm has unused variable and does not handle backward position movement

- Locations:
  - `src/compute/items.rs:124` to `src/compute/items.rs:134`
- Symptom:
  - `let _ref_pos = reference[i].session.normalized_car_position;` computes the reference position but never uses it.
  - The linear scan `for i in self.index..reference.len()` assumes `normalized_car_position` monotonically increases. If the car spins, goes off track, or moves backward, the position value can decrease. Starting the scan from `self.index` may skip valid earlier reference points or fall through to the error branch.
- Impact:
  - When the car position decreases relative to the previous frame, the algorithm returns an incorrect delta or fails with `ComputationFailed("无法在参考圈中找到对应位置")`.
  - This is a correctness defect in the dynamic example item that will manifest during real on-track incidents.
- Recommendation:
  - Remove the unused `_ref_pos` binding or use it to validate `current_pos >= ref_pos` when selecting the match interval.
  - Handle position regression: if `current_pos < reference[self.index].session.normalized_car_position`, reset `self.index` to 0 or scan backward before proceeding.
  - Add unit tests for backward movement and position-reset scenarios.

### P2: `DashboardService` advances schedule even for failed computations

- Locations:
  - `src/dashboard/service.rs:117` to `src/dashboard/service.rs:120`
- Symptom:
  - In `run()`, `next_schedule` is updated to `now + interval` for every name in `items_to_compute`.
  - This happens regardless of whether the item was found in `all_results` (e.g., the item failed or was not registered).
- Impact:
  - If a subscribed item fails (e.g., `DeltaToBestLap` with no reference data), the consumer waits a full interval before the next attempt, and the failure is invisible.
  - If an item name is misspelled during subscription, `all_results.get(name)` returns `None`, but the schedule is still advanced, causing the subscription to spin uselessly on every cycle.
- Recommendation:
  - Only advance `next_schedule` when computation succeeds and produces a result.
  - For failures, keep the previous schedule time (or use a shorter retry backoff) so the consumer can observe recovery.
  - Alternatively, change the sink protocol to include success/failure status so consumers can observe errors.

### P2: `CallbackSink` callback panic kills the dashboard thread without observation

- Locations:
  - `src/dashboard/sink.rs:70` to `src/dashboard/sink.rs:72`
- Symptom:
  - `CallbackSink::send()` calls `(self.callback)(data)` directly with no `std::panic::catch_unwind` guard.
- Impact:
  - Any panic in the callback unwinds through `DashboardService::run()`, killing the entire dashboard thread.
  - Because the CLI never checks the join handle (see P2 above), a bad callback silently and permanently disables the dashboard.
- Recommendation:
  - Wrap the callback invocation in `catch_unwind` and log/report the panic through an error channel instead of unwinding.
  - Alternatively, change `DataSink::send` to return a `Result` so `DashboardService` can handle send failures gracefully.

### P2: `ComputeRegistry::reference_cache` has no memory bounds or eviction

- Locations:
  - `src/compute/registry.rs:20`
  - `src/compute/registry.rs:112` to `src/compute/registry.rs:119`
- Symptom:
  - `reference_cache` is a `HashMap<ReferenceSource, Vec<TelemetryFrame>>`.
  - Each entry stores an entire lap of frames (potentially thousands of frames × size of `TelemetryFrame`).
  - `cache_reference_lap` allows insertion, but there is no size limit, TTL, or LRU eviction.
- Impact:
  - In a long-running `serve` or dashboard process, memory usage can grow without bound as new reference laps are loaded.
- Recommendation:
  - Add a maximum cache entry limit (e.g., `MAX_CACHE_ENTRIES`).
  - Or replace the `HashMap` with an LRU cache (e.g., `lru` crate) or a cache with TTL.
  - Expose `clear_reference_cache()` or `evict_reference(source)` API for explicit management.

### P3: `subscribe()` accepts unknown item names silently

- Location:
  - `src/dashboard/service.rs:57`
- Symptom:
  - The comment says item names must be registered.
  - The method does not check registration and returns `()`.
- Impact:
  - A typo creates a subscription that never emits data and produces no error.
- Recommendation:
  - Change signature to `subscribe(...) -> ComputeResult<()>`.
  - Return `ComputeError::ItemNotFound(name)` when the item is not registered.
  - Add a unit test for unknown subscription names.

### P3: Test files added for integration are placeholders

- Locations:
  - `tests/compute_tests.rs:1`
  - `tests/dashboard_tests.rs:1`
- Symptom:
  - Both files contain only comments.
  - Most tests live inside module `#[cfg(test)]` blocks.
- Impact:
  - Current tests cover isolated module behavior but not external API shape or CLI/dashboard wiring.
- Recommendation:
  - Add real integration tests for public API behavior:
    - registry item-targeted realtime computation
    - dashboard result output through a real receiver
    - unknown subscriptions
    - slow consumer policy
    - dynamic item with reference data

### P3: `TelemetryDistributor` documentation contradicts actual behavior

- Location:
  - `src/distributor.rs:38`
- Symptom:
  - The doc comment on `TelemetryDistributor::new()` states: "如果消费者处理速度跟不上，旧帧将被丢弃。"
  - The implementation uses `crossbeam_channel::bounded` with `try_send`. When the channel is full, `try_send` returns `Err`, meaning the **newest frame is dropped**, while old queued frames remain.
  - The unit test `test_overflow_drops_old` correctly asserts that frame 3 (new) is dropped and frames 1/2 (old) are kept.
- Impact:
  - Users expect the dashboard to receive the latest frame, but it actually processes stale queued frames while discarding newer ones.
  - This is a documentation/expectation mismatch that can mislead consumers and hide latency problems.
- Recommendation:
  - Update the doc comment to match the actual crossbeam `try_send` behavior (newest frame dropped when full).
  - Or (preferred) change the implementation to prefer the latest frame, aligning the behavior with the documented intent.

### P3: Errors in `compute_realtime` are only emitted via `eprintln!`

- Location:
  - `src/compute/registry.rs:84` to `src/compute/registry.rs:88`
- Symptom:
  - `ComputeRegistry::compute_realtime()` prints failures to stderr with `eprintln!("compute item '{}' failed: {err}; skipping", item.name())`.
  - The returned `HashMap` simply omits the failed item.
- Impact:
  - Programmatic callers (e.g., `DashboardService`) cannot distinguish between "item not subscribed" and "item failed to compute."
  - Production monitoring, alerting, and graceful degradation are impossible because failures are invisible to the API.
- Recommendation:
  - Include error information in the return value, for example `HashMap<String, ComputeResult<f64>>`, or maintain a parallel error log.
  - Or adopt the `compute_realtime_item` API (see P1 above) so callers receive typed errors directly.

### P3: `DashboardService::run()` uses frame arrival time as schedule baseline causing drift

- Locations:
  - `src/dashboard/service.rs:87`
  - `src/dashboard/service.rs:117` to `src/dashboard/service.rs:119`
- Symptom:
  - `now = Instant::now()` is captured when the frame arrives.
  - `next_schedule` is set to `now + interval`.
  - If frame processing and computation take time `dt`, the actual interval becomes `interval + dt`, and this drift accumulates across iterations.
- Impact:
  - High-frequency subscriptions (e.g., 50 ms) gradually deviate from the target cadence under load.
- Recommendation:
  - Update schedule based on the previous scheduled time: `next_schedule[name] += interval` instead of `now + interval`.
  - Or, when `now` is significantly later than `next_schedule + interval`, choose whether to catch up or skip missed intervals.

### P3: `serve_command` lacks graceful shutdown

- Location:
  - `src/bin/acc-live-telemetry.rs:1210` onward
- Symptom:
  - `serve` runs an infinite loop with no signal handling (SIGINT/SIGTERM).
  - On exit, the dashboard thread is not joined and resources are not explicitly flushed.
- Impact:
  - When the process is forcefully terminated, the dashboard thread may be left in an inconsistent state.
  - If dashboard later acquires persistent state, data loss could occur.
- Recommendation:
  - Add a simple Ctrl+C handler (e.g., using the `ctrlc` crate) to set a shutdown flag.
  - On shutdown, drop the distributor sender so the dashboard receiver disconnects, then `join` the dashboard handle before exiting.

## Recommended Fix Order

1. Close the dashboard output loop for `serve`/`record --dashboard`; make results observable.
2. Redesign realtime compute execution around explicit request/context parameters.
3. Make `DashboardService` execute only due subscribed items, plus explicit dependencies if needed.
4. Add reference-lap injection for dynamic realtime items.
5. Remove full-frame clone from the hot path or benchmark it and document the accepted cost.
6. Choose and implement a distributor overflow policy suitable for realtime dashboard data.
7. Fix `DeltaToBestLap` search robustness for backward position movement.
8. Prevent `CallbackSink` panic from killing the dashboard thread.
9. Add memory bounds or eviction to `reference_cache`.
10. Add integration tests for the end-to-end behavior rather than only module-local tests.

## Suggested API Direction

The following shape matches the clarified requirement that all required data be passed by the caller:

```rust
pub struct RealtimeComputeRequest<'a> {
    pub item_name: &'a str,
    pub current_frame: &'a TelemetryFrame,
    pub computed_values: &'a HashMap<String, f64>,
    pub reference_lap: Option<&'a [TelemetryFrame]>,
    pub reference_source: Option<ReferenceSource>,
}

impl ComputeRegistry {
    pub fn compute_realtime_item(
        &mut self,
        request: RealtimeComputeRequest<'_>,
    ) -> ComputeResult<f64> {
        // Find exactly request.item_name.
        // Build ComputeContext from the request.
        // Execute only that item.
        // Return its result or a typed error.
    }
}
```

For dashboard scheduling:

```rust
pub struct Subscription {
    pub item_name: String,
    pub interval: Duration,
    pub reference_source: Option<ReferenceSource>,
}
```

Dashboard should build a request per due subscription. If a reference lap is needed, it should resolve it explicitly and pass it into the request.

## Verification Performed

During this audit session, the following commands were run successfully before writing this report:

```powershell
cargo test
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
```

Results:

- `cargo test`: passed
- `cargo check --all-targets`: passed
- `cargo clippy --all-targets -- -D warnings`: passed

## Final Completion Record

All code fixes for the issues documented in this computed-items audit report were completed in commit:

- `b6f8436cebc68d02e188eb01a51ce7e0d4079187` (`fix(dashboard): complete computed items audit fixes`)

This includes the final R1 re-review correction for `DeltaToBestLap` position backtracking.

## Completion Commit

The code fixes for the remaining issues in this audit report were completed in:

- `b6f8436cebc68d02e188eb01a51ce7e0d4079187` (`fix(dashboard): complete computed items audit fixes`)

This commit includes the dashboard/compute/distributor/writer/test changes that close the audit items documented above, including the R1 re-review fix for `DeltaToBestLap` position backtracking.

Passing tests do not cover the P1/P2 integration issues above.

## Fix Log (2026-06-06)

All findings from this audit have been addressed. Each entry records the fix applied.

### P1: CLI dashboard output is computed but discarded

**Fix**: `DataSink::send()` changed to return `Result<(), SendError>` with typed `SendError` enum. CLI (`record --dashboard` and `serve`) now spawn a background thread reading `sink_rx` and printing results as `DASHBOARD speed_mps=27.7778` to stdout. `DashboardService` reports first send failure to stderr.

### P1: `DeltaToBestLap` cannot run through registry/dashboard path

**Fix**: Introduced `RealtimeComputeRequest<'a>` (in `context.rs`) carrying all computation inputs explicitly. Added `ComputeRegistry::compute_realtime()` overload taking `(name: &str, request: &RealtimeComputeRequest<'_>) -> ComputeResult<f64>` for per-item computation. Added `resolve_reference_lap()` with auto-load from `.acctlm` file when cache misses (Scheme B). Extended `DashboardService::subscribe()` to accept `Option<ReferenceSource>`. `run()` now builds per-item `RealtimeComputeRequest` with resolved reference lap. Conversion of `reference_cache` to `Arc<Vec<TelemetryFrame>>` avoids borrow conflicts.

### P2: Dashboard subscription scheduling executes all registered items

**Fix**: `DashboardService::run()` now calls `registry.compute_realtime(name, &request)` for each due item individually instead of `registry.compute_realtime(frame)` which executed all items. Only subscribed, due items are computed.

### P2: `record --dashboard` clones `TelemetryFrame` on 120 Hz hot path

**Fix**: `TelemetryDistributor::distribute()` now accepts `Arc<TelemetryFrame>` instead of `TelemetryFrame`. `BinaryTelemetryWriter::write_frame()` now accepts `&TelemetryFrame` (clones internally). `record_command` wraps frame in `Arc` once and shares between dashboard and writer.

### P2: Distributor drops newest frame; doc contradicts behavior

**Fix**: Updated `TelemetryDistributor::new()` doc from "旧帧将被丢弃" to "满时新帧被丢弃". Changed dashboard consumer capacity from 64 to 1 to minimize stale frame accumulation. Renamed test to `test_overflow_drops_new`.

### P2: Dashboard thread failures not observable

**Fix**: `_dashboard_handle` promoted to `dashboard_handle` with type annotation. Added `dashboard_dead` flag. `record_command` loop now checks `handle.is_finished()` every iteration; on death, disables distributor and prints warning.

### P2: `DeltaToBestLap` unused `_ref_pos`

**Fix**: Removed the unused `let _ref_pos = ...` line. Added comment clarifying that position back-tracking does not affect time delta computation.

### P2: `DashboardService` advances schedule for failed computations

**Fix**: Moved `next_schedule` update into `Ok` branch of `compute_realtime` match. Failed items retry on the next frame without waiting the full interval.

### P2: `CallbackSink` panic kills dashboard thread

**Fix**: `CallbackSink::send()` now wraps callback invocation in `std::panic::catch_unwind`. On panic, logs the message and returns `Ok(())`, keeping the dashboard thread alive.

### P2: `reference_cache` has no memory bounds

**Fix**: Added `MAX_CACHE_ENTRIES = 4`. `cache_reference_lap()` and `resolve_reference_lap()` both evict an existing entry before inserting when cache is full and key is new.

### P3: `subscribe()` accepts unknown item names silently

**Fix**: `DashboardService::subscribe()` now returns `ComputeResult<()>`. Validates `self.registry.is_registered(&item_name)` before inserting; returns `ComputeError::ItemNotFound` for unknown names. Added integration test `subscribe_unknown_item_returns_error`.

### P3: Test files are placeholders

**Fix**: Filled `tests/compute_tests.rs` with 3 integration tests (per-item computation, not-found error, cache eviction). Filled `tests/dashboard_tests.rs` with 3 integration tests (unknown subscription error, successful subscription, end-to-end data flow).

### P3: Distributor documentation contradicts behavior

**Fix**: See P2 fix above (doc updated, capacity reduced to 1).

### P3: Errors in `compute_realtime` only via `eprintln!`

**Fix**: Partially addressed — `DashboardService::run()` now uses `compute_realtime(name, &request)` which returns `ComputeResult<f64>`, making errors observable to callers. The legacy `compute_realtime(frame)` retains `eprintln!` for backward compatibility.

### P3: Schedule baseline uses frame arrival time causing drift

**Fix**: `DashboardService::run()` now uses `prev + interval` (previous scheduled time as baseline) instead of `now + interval` (frame arrival time as baseline), preventing cumulative drift.

### P3: `serve_command` lacks graceful shutdown

**Fix**: Added `ctrlc` dependency. `serve_command` registers a Ctrl+C handler that sets an `AtomicBool`. Main loop changed from `loop {` to `while running.load(SeqCst) {`. On shutdown, drops the distributor (disconnects dashboard receiver naturally), joins dashboard handle, and exits cleanly.

### Verification (post-fix)

```powershell
cargo test               # 41 tests pass (30 unit + 3 binary_roundtrip + 3 compute_tests + 3 dashboard_tests + 2 binary_roundtrip)
cargo check --all-targets # pass
cargo clippy --all-targets -- -D warnings # pass
```

## Re-review Finding (2026-06-06)

### R1: `DeltaToBestLap` backtracking fix is incomplete

- Location:
  - `src/compute/items.rs:108` to `src/compute/items.rs:116`
- Symptom:
  - The fix log states the `DeltaToBestLap` robustness issue was addressed.
  - The code only removed the unused `_ref_pos` binding and added a comment.
  - The algorithm still starts scanning from `self.index` even when the current car position has moved backward relative to the previously matched reference point.
- Concrete failure mode:
  - Reference positions: `0.0 -> 0.5 -> 1.0`.
  - First frame at current position `0.8` sets `self.index = 1`.
  - A later frame in the same lap at current position `0.4` still scans from index `1`.
  - It returns reference time at `0.5`, but the correct segment is before `0.5`, so it should reset/search from index `0`.
- Impact:
  - Backing up, sliding backward, or any position regression within the same lap can produce an incorrect time delta.
- Required fix:
  - If `current_pos` is less than `reference[self.index].session.normalized_car_position`, reset `self.index` to `0` before scanning.
  - Add a unit test that first advances the index, then feeds a lower `normalized_car_position` in the same lap and verifies the earlier reference point is used.

## Re-review Fix Log (2026-06-06)

### R1: `DeltaToBestLap` backtracking fix completed

**Fix**:

- Updated `DeltaToBestLap::compute()` so that if the current normalized position moves backward relative to the previously matched reference index, `self.index` is reset to `0` before scanning.
- This prevents the algorithm from matching against a later reference segment after backing up/sliding backward in the same lap.
- Added unit test `test_delta_to_best_lap_resets_index_on_position_backtrack`.

**Verification**:

```powershell
cargo test
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
```

Results:

- `cargo test`: passed
- `cargo check --all-targets`: passed
- `cargo clippy --all-targets -- -D warnings`: passed

## Final Completion Record

All code fixes for the issues documented in this computed-items audit report were completed in commit:

- `b6f8436cebc68d02e188eb01a51ce7e0d4079187` (`fix(dashboard): complete computed items audit fixes`)

This includes the final R1 re-review correction for `DeltaToBestLap` position backtracking.
