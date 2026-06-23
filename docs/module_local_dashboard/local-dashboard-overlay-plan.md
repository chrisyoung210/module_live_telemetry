# ACC Coach Local Dashboard Overlay Submodule Development Plan

## 1. Background and Goal

ACC Coach currently renders Local Dashboard inside the main desktop application window. That does not match the real user flow. During normal driving, users usually run Assetto Corsa Competizione (ACC) in fullscreen or near-fullscreen mode and cannot see the dashboard embedded inside ACC Coach.

This feature changes Local Dashboard into a standalone transparent always-on-top overlay shown above the ACC window.

Target behavior:

- When ACC enters live/running state, Local Dashboard overlay appears automatically.
- The overlay is an independent transparent Tauri window.
- The overlay stays above ACC in windowed or borderless-fullscreen style display modes.
- The overlay is click-through by default and does not interfere with ACC input.
- The overlay supports multiple regions. Each region chooses one registered dashboard layout and has its own screen anchor, offset, scale, and z-index.
- When ACC is paused, not live, disconnected, or its window cannot be found, the overlay hides automatically.
- Exclusive fullscreen is explicitly out of scope for guaranteed overlay display.

This document is written to be self-contained for a new VibeCoding session. Do not assume previous chat context.

## 2. High-Level Architecture

The feature should be developed as a submodule, not as scattered changes across unrelated dashboard and telemetry files.

Backend language and stack:

- Rust
- Tauri 2.11.1
- Windows API via `windows-sys`

Frontend language and stack:

- TypeScript
- React 18
- CSS Modules
- Tauri JavaScript API from `@tauri-apps/api`

The core design is one transparent overlay OS window containing multiple React-rendered regions. Do not create one OS window per dashboard layout.

Reasoning:

- One OS window is easier to keep always-on-top.
- One OS window is easier to hide/show atomically.
- Click-through behavior is simpler.
- React can position multiple dashboard regions internally.
- Future drag editing or profile switching remains possible.

## 3. Proposed File Organization

Backend module:

```text
src/local_dashboard_overlay/
  mod.rs
  config.rs
  window.rs
  acc_window.rs
  commands.rs
```

Backend responsibilities:

- `config.rs`: overlay config structs, default values, validation, JSON load/save.
- `window.rs`: Tauri overlay window creation, show/hide, always-on-top, click-through, bounds updates.
- `acc_window.rs`: Windows-only ACC top-level window detection and bounds lookup.
- `commands.rs`: Tauri commands exposed to frontend.
- `mod.rs`: public module exports and Tauri registration helper.

Frontend module:

```text
src-ui/features/local-dashboard-overlay/
  types.ts
  overlayConfigApi.ts
  telemetryFormat.ts
  dashboardRenderer.tsx
  LocalDashboardOverlay.tsx
  LocalDashboardOverlay.module.css
  OverlayRegionEditor.tsx
  OverlayRegionPreview.tsx
```

Frontend responsibilities:

- `types.ts`: frontend protocol types.
- `overlayConfigApi.ts`: typed wrappers around Tauri `invoke`.
- `telemetryFormat.ts`: dashboard value formatting, gear labels, delta labels, lap time labels.
- `dashboardRenderer.tsx`: shared renderer for static dashboard image plus dynamic controls.
- `LocalDashboardOverlay.tsx`: independent overlay window runtime.
- `OverlayRegionEditor.tsx`: main-window UI for editing regions.
- `OverlayRegionPreview.tsx`: main-window preview for overlay composition.

Existing `LocalDashboardView` should become the configuration surface. It should no longer pretend to run the dashboard inside the main window.

## 4. Runtime Model

### 4.1 Overlay Window

Create a hidden Tauri window with label:

```text
local-dashboard-overlay
```

Overlay URL:

```text
/?window=local-dashboard-overlay
```

Required window behavior:

```text
visible: false
decorations: false
transparent: true
alwaysOnTop: true
skipTaskbar: true
resizable: false
shadow: false
focus: false
clickThrough: true
```

If all properties cannot be expressed in `tauri.conf.json`, create the window during Rust setup with `WebviewWindowBuilder` and apply additional properties after creation.

The overlay window should be created once and reused. Hiding the overlay must not destroy the window.

### 4.2 Auto Trigger

Default trigger mode is auto-live.

Show overlay when all conditions are true:

```text
config.enabled == true
config.autoLive == true
autoRecordingStatus.live == true
autoRecordingStatus.paused == false
ACC window bounds are found
at least one region is enabled
```

Hide overlay when any condition is true:

```text
config.enabled == false
autoRecordingStatus.live == false
autoRecordingStatus.paused == true
shared memory is disconnected
ACC window bounds are not found
regions is empty or all regions are disabled
```

`autoLive` must be configurable because a future release may allow manual overlay control.

### 4.3 ACC Window Following

The overlay should follow the ACC window, not blindly cover the primary monitor.

Windows behavior:

- Enumerate top-level windows.
- Consider visible windows only.
- Read each window title.
- Match title containing `Assetto Corsa Competizione` or `ACC`.
- Read window rectangle.
- Reject tiny rectangles, for example width less than `800` or height less than `450`.
- Return `None` if no suitable window is found.

Overlay update behavior:

- Query ACC window bounds about every 500 ms.
- If bounds changed, move and resize the overlay window.
- If ACC window disappears, hide the overlay.
- Overlay internal coordinate system uses ACC window width and height.

## 5. Data Protocols

### 5.1 Overlay Config File

Config path:

```text
app_data_dir/local_dashboard_overlay.json
```

Schema name:

```text
acc-coach.local-dashboard-overlay.v1
```

Version:

```text
1
```

JSON shape:

```json
{
  "schema": "acc-coach.local-dashboard-overlay.v1",
  "version": 1,
  "enabled": true,
  "autoLive": true,
  "hideWhenNotLive": true,
  "followAccWindow": true,
  "clickThrough": true,
  "polling": {
    "statusMs": 500,
    "frameMs": 100,
    "windowMs": 500
  },
  "regions": [
    {
      "id": "region-delta",
      "name": "Time Delta",
      "enabled": true,
      "layoutId": "layout-example-delta",
      "anchor": "topLeft",
      "offsetX": 32,
      "offsetY": 32,
      "scale": 1.0,
      "zIndex": 10
    },
    {
      "id": "region-gear",
      "name": "Gear",
      "enabled": true,
      "layoutId": "layout-example-gear",
      "anchor": "bottomCenter",
      "offsetX": 0,
      "offsetY": -160,
      "scale": 1.0,
      "zIndex": 20
    }
  ]
}
```

Rust structs:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDashboardOverlayConfig {
    pub schema: String,
    pub version: u32,
    pub enabled: bool,
    pub auto_live: bool,
    pub hide_when_not_live: bool,
    pub follow_acc_window: bool,
    pub click_through: bool,
    pub polling: OverlayPollingConfig,
    pub regions: Vec<OverlayRegionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPollingConfig {
    pub status_ms: u64,
    pub frame_ms: u64,
    pub window_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayRegionConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub layout_id: String,
    pub anchor: OverlayAnchor,
    pub offset_x: f64,
    pub offset_y: f64,
    pub scale: f64,
    pub z_index: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}
```

Validation rules:

- `schema` must equal `acc-coach.local-dashboard-overlay.v1`.
- `version` must equal `1`.
- `polling.statusMs` clamps to `250..5000`.
- `polling.frameMs` clamps to `33..1000`.
- `polling.windowMs` clamps to `250..5000`.
- `region.scale` clamps to `0.1..5.0`.
- Empty region IDs should be generated before saving.
- A region with a missing `layoutId` should be preserved in config but skipped at runtime.

### 5.2 ACC Window Bounds Protocol

Tauri command:

```rust
#[tauri::command]
async fn get_acc_window_bounds() -> Result<Option<AccWindowBounds>, String>
```

Return shape:

```json
{
  "x": 0,
  "y": 0,
  "width": 2560,
  "height": 1440,
  "title": "Assetto Corsa Competizione",
  "matchedBy": "title"
}
```

TypeScript type:

```ts
export interface AccWindowBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  title: string;
  matchedBy: "title" | "fallback";
}
```

### 5.3 Overlay Window Commands

Tauri commands:

```rust
#[tauri::command]
async fn show_local_dashboard_overlay(app: tauri::AppHandle) -> Result<(), String>

#[tauri::command]
async fn hide_local_dashboard_overlay(app: tauri::AppHandle) -> Result<(), String>

#[tauri::command]
async fn set_local_dashboard_overlay_bounds(
    app: tauri::AppHandle,
    bounds: AccWindowBounds,
) -> Result<(), String>

#[tauri::command]
async fn set_local_dashboard_overlay_click_through(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String>
```

Rules:

- `show_local_dashboard_overlay` must reassert always-on-top before showing.
- `hide_local_dashboard_overlay` must not destroy the window.
- `set_local_dashboard_overlay_bounds` should use physical position and size to avoid DPI mismatch.
- `set_local_dashboard_overlay_click_through` defaults to enabled.

### 5.4 Config Commands

Tauri commands:

```rust
#[tauri::command]
async fn get_local_dashboard_overlay_config(
    app: tauri::AppHandle,
) -> Result<LocalDashboardOverlayConfig, String>

#[tauri::command]
async fn save_local_dashboard_overlay_config(
    app: tauri::AppHandle,
    config: LocalDashboardOverlayConfig,
) -> Result<LocalDashboardOverlayConfig, String>
```

Behavior:

- Load command creates default config if file does not exist.
- Save command normalizes and validates before writing.
- Invalid schema or version returns an error.
- Corrupt existing JSON should return an error and must not overwrite the file.

### 5.5 Live Session Command

Existing `start_live_session` returns an error if a live session already exists. Overlay runtime needs idempotent access.

Add:

```rust
#[tauri::command]
async fn ensure_live_session(
    live: tauri::State<'_, LiveState>,
    db: tauri::State<'_, Mutex<Database>>,
) -> Result<LiveSessionInfo, String>
```

Rules:

- If live session is not started, start it and return `LiveSessionInfo`.
- If live session is already started, return current session info.
- Do not break existing manual `LiveDashboard` start/stop behavior.

Conservative ownership rule for v1:

- Overlay may call `ensure_live_session`.
- Overlay should not call `stop_live_session`.
- Hiding overlay only hides the overlay window.
- This avoids accidentally stopping a live session started by the main UI.

## 6. Telemetry Data Protocol

Overlay uses existing `get_live_frame` command and existing `LiveFrame` data.

Frontend type should be defined inside the overlay submodule instead of imported from a component-local interface:

```ts
export interface LiveFrame {
  speedKmh: number;
  brakePct: number;
  throttlePct: number;
  gear: number;
  rpm: number;
  steeringDeg: number;
  distanceM: number;
  currentLapDistanceM: number | null;
  speedIntegratedLapDistanceM: number | null;
  lapNumber: number;
  position: number;
  sessionTimeLeftS: number;
  inPit: boolean;
  trackName: string | null;
  carModel: string | null;
  timestampMs: number;
  currentLapTimeMs: number | null;
  normalizedCarPosition: number | null;
  bestLapDeltaTimeMs: number | null;
  predictedLapTimeByBest: number | null;
  sessionLapDeltaTimeMs: number | null;
  predictedLapTimeBySession: number | null;
}
```

Dashboard dynamic fields should use the same field names used by dashboard output:

```text
speedKmh
gear
rpm
throttlePct
brakePct
steeringDeg
bestLapDeltaTimeMs
sessionLapDeltaTimeMs
predictedLapTimeByBest
predictedLapTimeBySession
currentLapTimeMs
currentLapDistanceM
sessionTimeLeftS
lapNumber
position
inPit
trackName
carModel
```

## 7. Frontend Rendering Rules

### 7.1 Entry Split

Current `src-ui/main.tsx` renders the main app directly. Change it to inspect query params:

```ts
const params = new URLSearchParams(window.location.search);
const windowKind = params.get("window");

if (windowKind === "local-dashboard-overlay") {
  render(<LocalDashboardOverlay />);
} else {
  render(<App />);
}
```

### 7.2 Region Positioning

Inputs:

```ts
containerWidth: number;
containerHeight: number;
layoutWidth: number;
layoutHeight: number;
scale: number;
anchor: OverlayAnchor;
offsetX: number;
offsetY: number;
```

Outputs:

```ts
left: number;
top: number;
width: layoutWidth * scale;
height: layoutHeight * scale;
```

Anchor algorithm:

```text
topLeft:
  baseX = 0
  baseY = 0

topCenter:
  baseX = (containerWidth - regionWidth) / 2
  baseY = 0

topRight:
  baseX = containerWidth - regionWidth
  baseY = 0

centerLeft:
  baseX = 0
  baseY = (containerHeight - regionHeight) / 2

center:
  baseX = (containerWidth - regionWidth) / 2
  baseY = (containerHeight - regionHeight) / 2

centerRight:
  baseX = containerWidth - regionWidth
  baseY = (containerHeight - regionHeight) / 2

bottomLeft:
  baseX = 0
  baseY = containerHeight - regionHeight

bottomCenter:
  baseX = (containerWidth - regionWidth) / 2
  baseY = containerHeight - regionHeight

bottomRight:
  baseX = containerWidth - regionWidth
  baseY = containerHeight - regionHeight
```

Final position:

```text
left = baseX + offsetX
top = baseY + offsetY
```

Do not clamp final position. Users may intentionally place a region partially off-screen.

### 7.3 Dashboard Layout Rendering

Each region renders one registered `DashboardLayoutPayload`.

Expected structure:

```tsx
<div className={styles.region} style={{ left, top, width, height, zIndex }}>
  <div
    className={styles.stage}
    style={{
      width: layout.canvasWidth,
      height: layout.canvasHeight,
      transform: `scale(${scale})`,
    }}
  >
    <img
      className={styles.staticImage}
      src={`data:${layout.imageMime};base64,${layout.staticImageBase64}`}
      alt=""
    />
    {layout.dynamicControls.map(control => (
      <DynamicDashboardControl control={control} frame={frame} />
    ))}
  </div>
</div>
```

Required CSS:

```css
.overlayRoot {
  position: fixed;
  inset: 0;
  background: transparent;
  pointer-events: none;
  overflow: hidden;
}

.region {
  position: absolute;
  pointer-events: none;
}

.stage {
  position: relative;
  transform-origin: top left;
}

.staticImage {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
}
```

### 7.4 Dynamic Control Text

Support:

- `{gear}`
- `{speedKmh}`
- `{value}` using `control.telemetryField`
- `{bestLapDeltaTimeMs|delta}`
- `{currentLapTimeMs|lapTime}`

Recommended format names:

```ts
type DashboardTextFormat =
  | "number"
  | "integer"
  | "delta"
  | "lapTime"
  | "percent"
  | "gear";
```

Formatting rules:

- `delta`: `+0.32s`, `-0.18s`, or `--`.
- `lapTime`: `1:48.326` or `--`.
- `percent`: `73%`.
- `integer`: rounded integer.
- `gear`: `R`, `N`, or positive gear number.
- unknown format: default `String(value)`.

Gear smoothing:

- Use `GEAR_DEBOUNCE_MS = 150`.
- If previous committed gear is positive and raw gear becomes `0`, hold previous gear for 150 ms.
- If raw gear becomes another positive gear within 150 ms, show the new positive gear directly.
- If raw gear remains `0` after 150 ms, show `N`.

### 7.5 Conditional Styles

Use existing rule shape:

```ts
interface DashboardConditionalRule {
  target: "textColor" | "backgroundColor" | string;
  telemetryField: string;
  operator: "gt" | "gte" | "lt" | "lte" | "eq" | "neq" | string;
  compareValue: number;
  color: string;
}
```

Rules:

- Only `textColor` and `backgroundColor` are applied in v1.
- Missing telemetry fields are ignored.
- Unknown operators are ignored.
- Later matching rules override earlier matching rules.

## 8. Main Window Configuration UI

`LocalDashboardView` becomes overlay configuration UI.

Required controls:

- Enable overlay toggle.
- Auto show when ACC live toggle.
- Click-through toggle.
- Follow ACC window toggle.
- Reload layouts button.
- Save config button.
- Add region button.

Each region editor supports:

- Name.
- Enabled.
- Layout selector.
- Anchor selector.
- Offset X.
- Offset Y.
- Scale.
- Z-index.
- Delete region.

The current in-main-window Start/Stop Local Dashboard behavior should be removed or replaced with config/status controls.

Initial default config:

- If at least one registered layout exists, create one default region:

```json
{
  "name": "Default Dashboard",
  "enabled": true,
  "layoutId": "first-registered-layout",
  "anchor": "bottomCenter",
  "offsetX": 0,
  "offsetY": -120,
  "scale": 1.0,
  "zIndex": 10
}
```

- If no registered layouts exist, regions should be empty.

## 9. Backend Implementation Steps

### 9.1 Add Windows API Features

Update Windows dependency features:

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.59", features = [
  "Win32_Foundation",
  "Win32_System_Memory",
  "Win32_UI_WindowsAndMessaging",
  "Win32_Graphics_Gdi"
] }
```

### 9.2 Implement Config Module

Implement:

```rust
impl LocalDashboardOverlayConfig {
    pub fn default() -> Self;
    pub fn load_or_create(path: &Path) -> Result<Self, String>;
    pub fn save(&self, path: &Path) -> Result<(), String>;
    pub fn normalized(self) -> Self;
}
```

Behavior:

- Missing file creates default config.
- Corrupt JSON returns error and does not overwrite.
- Unsupported schema/version returns error.
- Valid config is normalized before saving.

### 9.3 Implement ACC Window Module

Windows:

```rust
pub fn find_acc_window_bounds() -> Result<Option<AccWindowBounds>, String>
```

Non-Windows:

```rust
pub fn find_acc_window_bounds() -> Result<Option<AccWindowBounds>, String> {
    Ok(None)
}
```

### 9.4 Implement Overlay Window Module

Create:

```rust
pub fn ensure_overlay_window(app: &tauri::AppHandle) -> Result<(), String>
```

This should:

- Return existing window if present.
- Create hidden overlay window if missing.
- Apply transparent/no-decoration/always-on-top/skip-taskbar/click-through settings.

### 9.5 Register Commands

Add commands to the existing Tauri invoke handler:

```rust
get_local_dashboard_overlay_config
save_local_dashboard_overlay_config
get_acc_window_bounds
show_local_dashboard_overlay
hide_local_dashboard_overlay
set_local_dashboard_overlay_bounds
set_local_dashboard_overlay_click_through
ensure_live_session
```

If the project keeps all commands in `src/ipc/mod.rs`, wire the overlay module commands into that existing handler without creating a second conflicting `invoke_handler`.

## 10. Frontend Implementation Steps

### 10.1 Extract Shared Dashboard Renderer

Extract reusable logic from existing dashboard views:

- `cssColor`
- telemetry text rendering
- fallback text
- conditional rule evaluation
- dynamic control rendering
- custom font injection

Overlay and preview should use the same renderer to avoid different behavior.

### 10.2 Implement LocalDashboardOverlay

State:

```ts
const [config, setConfig] = useState<LocalDashboardOverlayConfig | null>(null);
const [layouts, setLayouts] = useState<RegisteredDashboardLayout[]>([]);
const [status, setStatus] = useState<AutoRecordingStatus | null>(null);
const [frame, setFrame] = useState<LiveFrame | null>(null);
const [bounds, setBounds] = useState<AccWindowBounds | null>(null);
const [visible, setVisible] = useState(false);
```

Polling:

- status polling: default 500 ms.
- ACC window polling: default 500 ms.
- frame polling while visible: default 100 ms.

State transitions:

```text
idle
  -> waitingForAccWindow
  -> visible
  -> hidden
```

Show flow:

1. Load config.
2. Load registered layouts.
3. Check `status.live && !status.paused`.
4. Query ACC bounds.
5. Call `set_local_dashboard_overlay_bounds`.
6. Call `show_local_dashboard_overlay`.
7. Start frame polling.
8. Render enabled regions.

Hide flow:

1. Stop frame polling.
2. Call `hide_local_dashboard_overlay`.
3. Keep config/layouts loaded for fast re-show.

### 10.3 Update LocalDashboardView

Replace current local runtime preview with overlay config editing.

Keep:

- registered layout list
- selected layout preview
- reload layouts

Add:

- overlay config load/save
- region editor
- region composition preview
- runtime status summary
- validation warnings

Avoid long explanatory copy in the UI. Use concise labels and controls.

## 11. Testing Plan

### 11.1 Rust Tests

Cover:

- default config serialization.
- `load_or_create` when file is missing.
- invalid schema.
- invalid version.
- corrupt JSON does not overwrite.
- polling clamp.
- scale clamp.
- empty region ID normalization.
- non-Windows `find_acc_window_bounds` returns `Ok(None)`.

### 11.2 Frontend Tests

Use Vitest.

Cover:

- anchor positioning for all anchors.
- offset and scale positioning.
- `{value}` resolution through `telemetryField`.
- `{gear}` label formatting.
- gear smoothing behavior.
- delta formatting.
- lap time formatting.
- conditional color rule application.
- missing layout skips region.
- disabled region does not render.

### 11.3 Manual Verification

Run:

```text
npm run build
cargo check
```

Manual scenarios:

1. No registered layouts: Local Dashboard config page opens and allows no active region.
2. One registered layout: user can add region and save config.
3. Multiple registered layouts: user can assign different layouts to different regions.
4. ACC not running: overlay stays hidden.
5. ACC running but not live: overlay stays hidden.
6. ACC live/running: overlay appears.
7. ACC paused: overlay hides.
8. ACC exits: overlay hides.
9. Windowed fullscreen / borderless: overlay is visible above ACC.
10. Exclusive fullscreen: documented as unsupported or not guaranteed.
11. Overlay click-through does not steal focus.
12. Moving ACC window causes overlay to follow.
13. Gear region at bottom center updates correctly.
14. Time delta region at top left updates correctly.
15. Gear does not flicker to `N` during normal shifts.

## 12. Development Standards

### 12.1 Rust Standards

- Keep overlay code in its own module.
- Public API should be minimal.
- Frontend-facing structs use `#[serde(rename_all = "camelCase")]`.
- Tauri commands return `Result<T, String>`.
- Error strings must be readable.
- Windows-specific code must be guarded by `#[cfg(windows)]`.
- Non-Windows builds must still compile.
- Do not panic inside commands.
- Do not block the UI thread with long polling work.
- Do not modify auto recording logic unless adding read-only state or idempotent live access.

### 12.2 TypeScript Standards

- Define overlay protocol types in the overlay feature module.
- Do not import component-local interfaces from unrelated components.
- All intervals must be cleaned up in `useEffect` cleanup.
- Do not call `invoke` during render.
- Use CSS Modules.
- Overlay root must use `pointer-events: none`.
- Use stable region IDs as React keys.
- Clamp numeric inputs before saving.
- Do not use array index as region key.

### 12.3 Data Compatibility Standards

- `local_dashboard_overlay.json` must include schema and version.
- v1 does not need migration.
- Unknown schema/version returns error.
- Existing registered dashboard layout protocol must not be changed:
  - `DashboardLayoutPayload`
  - `RegisteredDashboardLayout`
  - `DashboardControl`
- Overlay config references layouts by `layoutId`.
- Overlay config does not copy layout payloads.
- Missing layout at runtime means skip that region, not delete it.

## 13. Acceptance Criteria

Functional:

- User can configure multiple overlay regions.
- User can place time delta in top-left.
- User can place gear in lower center.
- ACC live state automatically shows overlay.
- Non-live, paused, disconnected, or missing ACC window automatically hides overlay.
- Overlay stays above ACC in windowed or borderless-fullscreen mode.
- Overlay is click-through.
- Dynamic telemetry values update in real time.
- Static dashboard image and dynamic controls both render correctly.

Engineering:

- `npm run build` passes.
- `cargo check` passes.
- Config JSON is persisted and reloads correctly.
- Existing remote/Android dashboard protocol is not broken.
- Existing manual LiveDashboard start/stop is not broken.
- Existing auto recording is not broken.

## 14. Explicit Non-Goals for v1

Do not implement in v1:

- Guaranteed exclusive fullscreen overlay.
- Dragging overlay regions directly on the game screen.
- Multiple OS overlay windows.
- Manual multi-monitor selection.
- Automatic ACC UI scaling detection.
- Complex overlay profiles.
- In-game hotkeys for overlay toggle.
- Large dashboard designer rewrite.
- Android remote dashboard protocol changes.

## 15. Default Decisions

- Multiple layout model: one overlay window with multiple regions.
- Trigger: auto recording status `live && !paused`.
- Hide policy: hide automatically when not live.
- Input policy: click-through.
- Screen policy: follow ACC window; hide if ACC window is not found.
- Initial positioning: anchor plus offset plus scale.
- Backend language: Rust.
- Frontend language: TypeScript React.
- Data protocol: versioned JSON config.
