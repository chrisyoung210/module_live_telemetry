# Dashboard `raw:controls.speed_kmh` Live Trace Findings

Date: 2026-06-14

## Context

While using the live dashboard, `raw:controls.speed_kmh` appeared to show a very large value, roughly a 10-digit number. Directly reading/parsing the generated `.acctlm2` file showed normal speed values.

To verify whether the bad value was produced by `module_live_telemetry` or by the caller/UI layer, a temporary sampled CSV trace was added inside `DashboardService`.

## Trace Location

Without environment variables, the trace is written to:

```text
C:\Users\congj\AppData\Local\Temp\acc-dashboard-values.csv
```

The trace writes one CSV row per dashboard item for sampled dashboard frames:

```text
wall_time_ms,sent_frame_count,sample_tick,timestamp_ns,value_count,item,value
```

Default sampling behavior:

- First successfully sent dashboard frame is always written.
- Then every 60 successfully sent dashboard frames are written.
- This is controlled by `ACC_DASHBOARD_TRACE_GAP`.
- The path is controlled by `ACC_DASHBOARD_TRACE_PATH`.
- The trace can be disabled with `ACC_DASHBOARD_TRACE=0`.

## Evidence From Current Trace

Filtering the CSV to only `item == raw:controls.speed_kmh` showed normal speed values.

Summary from the captured file:

```text
count = 55
min   = 0
max   = 178.591583251953
avg   = 53.0725410883902
```

Example rows:

```text
sent_frame_count,sample_tick,timestamp_ns,value_count,item,value
2640,2639,22536218800,11,raw:controls.speed_kmh,178.59158325195313
2760,2759,23559915200,11,raw:controls.speed_kmh,131.55360412597656
2820,2819,24071495800,11,raw:controls.speed_kmh,126.81326293945313
3180,3179,27150445100,11,raw:controls.speed_kmh,100.62183380126953
```

There were no `raw:controls.speed_kmh` rows greater than `1000` in the captured CSV.

However, the same CSV rows contain 10-digit or 11-digit `timestamp_ns` values, for example:

```text
sample_tick=2759
timestamp_ns=23559915200
raw:controls.speed_kmh=131.55360412597656
```

This means the 10-digit number observed in the UI is consistent with `timestamp_ns`, not with the dashboard value for `raw:controls.speed_kmh`.

## Current Conclusion

`module_live_telemetry` appears to be producing and sending the correct value for:

```text
raw:controls.speed_kmh
```

The likely issue is downstream of `DashboardValuesFrame`, in the caller/UI/dashboard rendering layer. Specifically, a caller may be:

- Reading the wrong field from the returned dashboard frame.
- Displaying `timestampNs` / `timestamp_ns` where the selected telemetry item value should be displayed.
- Confusing a frame metadata field with an entry in `DashboardValuesFrame.values`.
- Misinterpreting CSV columns during manual inspection.

## Relevant Producer-Side API Shape

The live telemetry dashboard sends:

```rust
pub struct DashboardValuesFrame {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub values: HashMap<String, f64>,
}
```

For a selected item such as:

```text
raw:controls.speed_kmh
```

the value must be read from:

```rust
frame.values["raw:controls.speed_kmh"]
```

It should not be read from:

```rust
frame.timestamp_ns
```

or from any top-level frame metadata field.

## Caller-Side Paths Observed In `acc-coach`

The live monitor stores the dashboard frame here:

```text
D:\WorkSpaces\VibeCoding\racing_team\acc-coach\src\recording\auto.rs
```

Important structure:

```rust
pub struct AutoDashboardFrame {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub values: HashMap<String, f64>,
}
```

The IPC endpoint that exposes the latest frame to the frontend is:

```text
D:\WorkSpaces\VibeCoding\racing_team\acc-coach\src\ipc\mod.rs
```

Relevant function:

```rust
get_live_dashboard_frame(...)
```

It currently does the right general shape:

```rust
let mut fields = recording_dashboard_fields(frame.values);
fields.insert("sampleTick".to_string(), json!(frame.sample_tick));
fields.insert("timestampNs".to_string(), json!(frame.timestamp_ns));
fields.insert("timestampMs".to_string(), json!(frame.timestamp_ns as f64 / 1_000_000.0));
```

The conversion function is:

```text
D:\WorkSpaces\VibeCoding\racing_team\acc-coach\src\recording\writer.rs
```

Relevant function:

```rust
recording_dashboard_fields(...)
```

It preserves both the raw key and a friendly alias:

```rust
"raw:controls.speed_kmh" => "speedKmh"
```

So the frontend should be able to read either:

```text
raw:controls.speed_kmh
```

or:

```text
speedKmh
```

Both should resolve to the same normal speed value.

## Current Saved Overlay Configuration

The active local dashboard overlay configuration references this layout:

```text
C:\Users\congj\AppData\Roaming\com.acc-coach.desktop\local_dashboard_overlay.json
```

Active region:

```json
{
  "layoutId": "layout-1781362123260"
}
```

The corresponding dashboard layout contains this dynamic control:

```text
C:\Users\congj\AppData\Roaming\com.acc-coach.desktop\dashboard_layouts.json
```

```json
{
  "id": "speed",
  "text": "{raw:controls.speed_kmh|0}",
  "telemetryField": null
}
```

This means the renderer should resolve the explicit placeholder field:

```text
raw:controls.speed_kmh
```

from the frame object returned by `get_live_dashboard_frame`.

## Frontend Rendering Path Observed

The local overlay frontend polls:

```text
D:\WorkSpaces\VibeCoding\racing_team\acc-coach\src-ui\components\LocalDashboardOverlayWindow.tsx
```

Relevant call:

```ts
invoke<LiveFrame | null>("get_live_dashboard_frame")
```

Rendering is delegated to:

```text
D:\WorkSpaces\VibeCoding\racing_team\module_local_dashboard\src-ui\features\local-dashboard-overlay\dashboardRenderer.tsx
```

Relevant function:

```ts
function readFrameValue(frame: LiveFrame, field: string): unknown {
  return (frame as unknown as Record<string, unknown>)[field];
}
```

This direct object lookup should work for keys with colon/dot characters, such as:

```text
raw:controls.speed_kmh
```

provided the returned IPC object actually contains that exact key.

## Secondary Issue Found

The active layout uses:

```text
{raw:controls.speed_kmh|0}
```

But the current formatter only recognizes named formats such as:

```text
number
integer
delta
lapTime
percent
gear
```

It does not currently treat `"0"` as a decimal-place format. Therefore `{raw:controls.speed_kmh|0}` may be rendered using the raw string value, e.g.:

```text
126.81326293945313
```

instead of:

```text
127
```

This explains too many decimals, but it does not explain a 10-digit value.

Suggested fix:

- Treat numeric placeholder formats like `0`, `0.0`, `0.00`, etc. as decimal-place formatting.
- Or change the layout text to use an existing named format if supported by the designer, such as `integer`.

## Recommended Caller-Side Debug Logging

Add temporary logging at the IPC boundary in `get_live_dashboard_frame`, immediately before returning `fields`.

Recommended fields to log:

```rust
let raw_speed = fields.get("raw:controls.speed_kmh").cloned();
let speed_kmh = fields.get("speedKmh").cloned();
let timestamp_ns = fields.get("timestampNs").cloned();

eprintln!(
    "live dashboard frame: raw_speed={:?} speedKmh={:?} timestampNs={:?} keys={:?}",
    raw_speed,
    speed_kmh,
    timestamp_ns,
    fields.keys().collect::<Vec<_>>()
);
```

Expected result:

```text
raw_speed = ~0..300
speedKmh = ~0..300
timestampNs = large nanosecond counter
```

If `raw_speed` and `speedKmh` are normal here, the bug is in frontend rendering or UI field selection.

If one of them is missing or null, the bug is in subscription setup or field conversion.

If `raw_speed` is already a 10-digit number here, the bug is before IPC, and the next place to inspect is `recording_dashboard_fields(frame.values)`.

## Recommended Frontend Debug Logging

Add temporary logging in:

```text
D:\WorkSpaces\VibeCoding\racing_team\module_local_dashboard\src-ui\features\local-dashboard-overlay\dashboardRenderer.tsx
```

Inside `resolveControlText`, after resolving `field`:

```ts
if (field === "raw:controls.speed_kmh" || field === "speedKmh") {
  console.log("dashboard speed render", {
    field,
    value: readFrameValue(frame, field),
    speedKmh: frame ? (frame as unknown as Record<string, unknown>).speedKmh : undefined,
    rawSpeed: frame ? (frame as unknown as Record<string, unknown>)["raw:controls.speed_kmh"] : undefined,
    timestampNs: frame ? (frame as unknown as Record<string, unknown>).timestampNs : undefined,
  });
}
```

Expected result:

```text
field = "raw:controls.speed_kmh"
value = normal speed
timestampNs = large nanosecond counter
```

If `value` equals `timestampNs`, inspect the frontend field selection or any transformation between IPC result and `LocalDashboardOverlay`.

## Recommended Manual Checks

1. Confirm the CSV value column, not the `timestamp_ns` column:

```powershell
Import-Csv C:\Users\congj\AppData\Local\Temp\acc-dashboard-values.csv |
  Where-Object { $_.item -eq 'raw:controls.speed_kmh' } |
  Select-Object sent_frame_count,sample_tick,timestamp_ns,item,value
```

2. Confirm no speed rows are huge:

```powershell
Import-Csv C:\Users\congj\AppData\Local\Temp\acc-dashboard-values.csv |
  Where-Object { $_.item -eq 'raw:controls.speed_kmh' -and [double]$_.value -gt 1000 }
```

3. Compare one sampled row:

```text
timestamp_ns should be large.
value for raw:controls.speed_kmh should be normal.
```

## Summary

The captured producer-side trace shows:

- `raw:controls.speed_kmh` is normal in `DashboardValuesFrame.values`.
- The observed 10-digit number matches `timestamp_ns`, not speed.
- The likely bug is in caller/UI value selection, IPC transformation, or display logic.
- There is also a smaller formatter issue with `{raw:controls.speed_kmh|0}` not being interpreted as integer formatting.

