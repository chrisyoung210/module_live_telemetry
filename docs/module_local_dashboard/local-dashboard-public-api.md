# Local Dashboard Overlay Public API

This module is a code-level submodule for ACC Coach. It is not a standalone
application, does not own a dev server, and does not own live-session state.

The main module owns lifecycle orchestration:

- when overlay config is loaded or saved
- when ACC/live/session state is polled
- when telemetry frames are fetched
- when the overlay window is shown, hidden, moved, or resized
- how Tauri IPC commands are registered
- how the existing `LocalDashboardView` integrates the editor and preview

This module provides reusable Rust helpers, shared protocol types, and
controlled React components.

## Rust Crate

Crate name:

```toml
acc-coach-local-dashboard-overlay
```

Library import path:

```rust
acc_coach_local_dashboard_overlay
```

Recommended workspace dependency:

```toml
acc-coach-local-dashboard-overlay = { path = "../module_local_dashboard" }
```

## Rust Module Boundary

Public module:

```rust
pub mod local_dashboard_overlay;
```

Crate-root re-exports:

```rust
pub use local_dashboard_overlay::{
    acc_window::{AccWindowBounds, AccWindowMatchedBy},
    config::{
        LocalDashboardOverlayConfig,
        OverlayAnchor,
        OverlayPollingConfig,
        OverlayRegionConfig,
        OVERLAY_CONFIG_SCHEMA,
        OVERLAY_CONFIG_VERSION,
    },
};
```

## Rust Setup Helper

```rust
pub fn local_dashboard_overlay::setup(app: &tauri::App) -> Result<(), String>
```

Creates or reuses the transparent overlay Tauri window.

Behavior:

- creates one window with label `local-dashboard-overlay`
- uses URL `/?window=local-dashboard-overlay`
- starts hidden
- sets no decorations, transparent background, no shadow, not resizable
- reasserts always-on-top and click-through flags

The main module decides when to call this. A typical place is the main Tauri
`setup` callback.

## Rust Config API

Constants:

```rust
pub const OVERLAY_CONFIG_SCHEMA: &str = "acc-coach.local-dashboard-overlay.v1";
pub const OVERLAY_CONFIG_VERSION: u32 = 1;
```

Types:

```rust
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

pub struct OverlayPollingConfig {
    pub status_ms: u64,
    pub frame_ms: u64,
    pub window_ms: u64,
}

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

Methods:

```rust
impl LocalDashboardOverlayConfig {
    pub fn load_or_create(path: &Path) -> Result<Self, String>;
    pub fn save(&self, path: &Path) -> Result<(), String>;
    pub fn normalized(self) -> Self;
}
```

Config behavior:

- missing file creates and saves default config
- corrupt JSON returns an error and does not overwrite the file
- unsupported schema or version returns an error
- `status_ms` clamps to `250..5000`
- `frame_ms` clamps to `33..1000`
- `window_ms` clamps to `250..5000`
- region `scale` clamps to `0.1..5.0`
- empty region IDs are generated during normalization
- empty region names become `Dashboard Region`

The main module owns the final config path. The planned default is:

```text
app_data_dir/local_dashboard_overlay.json
```

## Rust ACC Window API

```rust
pub fn local_dashboard_overlay::acc_window::find_acc_window_bounds(
) -> Result<Option<AccWindowBounds>, String>
```

Return type:

```rust
pub struct AccWindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub title: String,
    pub matched_by: AccWindowMatchedBy,
}

pub enum AccWindowMatchedBy {
    Title,
    Fallback,
}
```

Windows behavior:

- enumerates top-level visible windows
- matches titles containing `Assetto Corsa Competizione` or `ACC`
- rejects tiny windows smaller than `800x450`
- returns `Ok(None)` when no suitable window is found

Non-Windows behavior:

- returns `Ok(None)`

## Rust Overlay Window API

Constants:

```rust
pub const OVERLAY_WINDOW_LABEL: &str = "local-dashboard-overlay";
```

Functions:

```rust
pub fn local_dashboard_overlay::window::ensure_overlay_window(
    app: &tauri::AppHandle,
) -> Result<(), String>;

pub fn local_dashboard_overlay::window::show_overlay_window(
    app: &tauri::AppHandle,
) -> Result<(), String>;

pub fn local_dashboard_overlay::window::hide_overlay_window(
    app: &tauri::AppHandle,
) -> Result<(), String>;

pub fn local_dashboard_overlay::window::set_overlay_bounds(
    app: &tauri::AppHandle,
    bounds: &AccWindowBounds,
) -> Result<(), String>;

pub fn local_dashboard_overlay::window::set_overlay_click_through(
    app: &tauri::AppHandle,
    enabled: bool,
) -> Result<(), String>;
```

Ownership:

- these are low-level helpers
- they do not poll, debounce, or decide lifecycle
- the main module decides when to call them
- hiding the overlay does not destroy the window

## Rust Non-API

This module intentionally does not expose Tauri commands.

The main module may wrap the Rust helpers in IPC commands if that fits the
host architecture, but command names, registration, permissions, and state
ownership belong to the main module.

This module does not implement:

- `ensure_live_session`
- `get_live_frame`
- `get_auto_recording_status`
- `get_registered_dashboard_layouts`
- auto-recording ownership
- shared-memory connection ownership
- dashboard layout registry ownership

## Frontend Public Entry

Frontend exports are available from:

```text
src-ui/features/local-dashboard-overlay/index.ts
```

Exports:

```ts
export * from "./dashboardRenderer";
export * from "./LocalDashboardOverlay";
export * from "./OverlayRegionEditor";
export * from "./OverlayRegionPreview";
export * from "./telemetryFormat";
export * from "./types";
```

The frontend module does not call Tauri `invoke`. It is controlled by the main
module through props and callbacks.

## Frontend Protocol Types

Primary config types:

```ts
export interface LocalDashboardOverlayConfig {
  schema: "acc-coach.local-dashboard-overlay.v1";
  version: 1;
  enabled: boolean;
  autoLive: boolean;
  hideWhenNotLive: boolean;
  followAccWindow: boolean;
  clickThrough: boolean;
  polling: OverlayPollingConfig;
  regions: OverlayRegionConfig[];
}
```

Telemetry frame:

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

Dashboard layout types:

```ts
export interface RegisteredDashboardLayout {
  id: string;
  name: string;
  payload: DashboardLayoutPayload;
}

export interface DashboardLayoutPayload {
  canvasWidth: number;
  canvasHeight: number;
  imageMime: string;
  staticImageBase64: string;
  dynamicControls: DashboardControl[];
}
```

These types mirror existing ACC Coach dashboard data. This module references
layouts by `layoutId`; it does not own layout registration or persistence.

## LocalDashboardOverlay Component

```ts
export interface LocalDashboardOverlayProps {
  config: LocalDashboardOverlayConfig;
  containerWidth: number;
  containerHeight: number;
  frame: LiveFrame | null;
  gearState?: GearSmootherState;
  layouts: RegisteredDashboardLayout[];
  visible: boolean;
}
```

Usage:

```tsx
<LocalDashboardOverlay
  config={config}
  containerWidth={accBounds.width}
  containerHeight={accBounds.height}
  frame={liveFrame}
  gearState={gearState}
  layouts={registeredLayouts}
  visible={overlayVisible}
/>
```

Behavior:

- renders nothing when `visible` is `false`
- renders enabled regions only
- skips regions with missing `layoutId`
- skips regions whose layout is not present in `layouts`
- uses `containerWidth` and `containerHeight` as the overlay coordinate system
- does not fetch config, status, layouts, or telemetry
- does not show, hide, move, or resize the OS window

## OverlayRegionEditor Component

Controlled editor for one `OverlayRegionConfig`.

The main module owns the region list and passes:

- available anchors
- available layouts
- current region
- `onChange`
- `onDelete`

This component does not save config by itself.

## OverlayRegionPreview Component

Composition preview for the main-window configuration surface.

Inputs:

- `config`
- `layouts`
- preview `width`
- preview `height`

This component uses a sample frame internally. It is intended for layout
composition preview, not live telemetry playback.

## Renderer API

Important exports from `dashboardRenderer`:

```ts
computeRegionRect(input): RegionRect
DashboardRegionRenderer(props)
DynamicDashboardControl(props)
resolveControlText(control, frame, gearState?)
computeControlStyle(control, frame)
evaluateConditionalRules(rules, frame)
```

`computeRegionRect` implements anchor + offset + scale positioning. It does
not clamp the final position.

Conditional styles support:

- `textColor`
- `backgroundColor`

Unknown fields, targets, and operators are ignored.

## Telemetry Formatting API

Important exports from `telemetryFormat`:

```ts
export const GEAR_DEBOUNCE_MS = 150;

formatTelemetryValue(value, format): string
formatDelta(value): string
formatLapTime(value): string
formatGear(rawGear): string
createInitialGearSmootherState(initialGear?): GearSmootherState
smoothGear(state, rawGear, timestampMs, debounceMs?): GearSmootherState
```

Formatting behavior:

- `delta`: `+0.32s`, `-0.18s`, or `--`
- `lapTime`: `1:48.326` or `--`
- `percent`: rounded whole percent
- `integer`: rounded integer
- `gear`: `R`, `N`, or positive gear number

Gear smoothing is opt-in. The main module owns the persisted smoother state and
passes it to `LocalDashboardOverlay`.

## Main Module Responsibilities

The main module should:

- register this crate as a workspace member or path dependency
- call `setup` or `ensure_overlay_window` during Tauri setup
- decide how to expose config/window helpers through its own architecture
- own live-session state
- own ACC shared-memory connection state
- own telemetry frame polling
- own dashboard layout registration
- own overlay show/hide policy
- own bounds-following policy
- own config save/load UI workflow
- route the overlay window URL to `LocalDashboardOverlay`

## Stability Notes

Stable public surface for v1:

- Rust config structs and constants
- Rust ACC window bounds type
- Rust overlay window helper functions
- TypeScript protocol types
- controlled React components
- renderer and formatter helpers covered by tests

Non-stable implementation details:

- CSS class names
- sample preview frame values
- exact internal JSX structure
- test-only utility structure

## Dashboard Widget Type Contract (v2)

This section defines the widget type system for dashboard controls. It replaces the old `isDynamic` boolean with an explicit `widgetType` enum and adds contracts for `chart` and `map` widgets. All types referenced below are imported from `module_dashboard_protocol/types`.

### 1. WidgetType Support

`DashboardControl.widgetType` accepts the following values:

```ts
type WidgetType = "static" | "text" | "chart" | "map";
```

- `"static"` — renders the region background image only
- `"text"` — renders a formatted telemetry value using `telemetryFormat`
- `"chart"` — renders a multi-field time-series line chart
- `"map"` — renders a track outline with a car position dot

Old `isDynamic` maps to `widgetType` as follows:

| Old field | Value | Maps to |
|-----------|-------|---------|
| `isDynamic` | `true` | `"text"` |
| `isDynamic` | `false` | `"static"` |
| absent | — | `"text"` |

When `widgetType` is already present, it takes priority and `isDynamic` is ignored.

### 2. Chart Widget Contract

A chart widget displays one or more telemetry fields as overlayed line charts over a fixed time window.

**Control fields:**

```ts
chartFields: ChartFieldConfig[];
chartWindowS: number;
```

`ChartFieldConfig` shape (from `module_dashboard_protocol`):

```ts
interface ChartFieldConfig {
  fieldName: string;
  color: string;
  label: string;
}
```

**Rendering contract:**

- Canvas 2D line chart
- Y axis range is fixed to `0`–`1`
- Multiple fields are drawn as overlaid polylines, each with its own color
- Data is presented as a ring buffer: only points inside `chartWindowS` seconds are visible
- If no data is available, the widget shows "No data"

**Props injection:**

The main module injects history data through props. The renderer receives:

```ts
history: FieldHistory[];

interface FieldHistory {
  field_name: string;
  points: { t: number; v: number }[];
}
```

The chart widget **must not** call Tauri `invoke` to fetch history. All data arrives through the `history` prop.

### 3. Map Widget Contract

A map widget displays a track outline and the current car position.

**Control fields:**

```ts
trackId: string;
dotColor: string;
dotSize: number;
```

**Rendering contract:**

- Canvas 2D track outline drawn as a polyline
- Car position shown as a colored dot on the track
- Aspect ratio is preserved; the track is centered and scaled to fit
- If no track data is available, the widget shows "No track data"

**Props injection:**

The main module injects track geometry through props:

```ts
trackPoints: Record<string, { x: number; z: number }[]>;
```

The map widget looks up its points by `trackId`. It **must not** call Tauri `invoke` to fetch track data.

### 4. DashboardValuesFrame Props

`LocalDashboardOverlay` receives telemetry through `DashboardValuesFrame` instead of the old `LiveFrame`.

**New props:**

```ts
export interface LocalDashboardOverlayProps {
  config: LocalDashboardOverlayConfig;
  containerWidth: number;
  containerHeight: number;
  frame: DashboardValuesFrame | null;
  history: FieldHistory[];
  trackPoints: Record<string, { x: number; z: number }[]>;
  gearState?: GearSmootherState;
  layouts: RegisteredDashboardLayout[];
  visible: boolean;
}
```

`DashboardValuesFrame` shape (from `module_dashboard_protocol`):

```ts
interface DashboardValuesFrame {
  subscriptionGeneration: number;
  sampleTick: number;
  timestampNs: number;
  values: Record<string, number>;
}
```

- `values` is a flat record keyed by telemetry channel id (e.g. `raw:controls.speed_kmh`)
- `subscriptionGeneration` increments when the subscription set changes
- Text widgets read their values from `frame.values[control.telemetryField]`
- Chart widgets read their history from the `history` prop
- Map widgets read their track from the `trackPoints` prop

### 5. dashboardRenderer widgetType Dispatch

`DynamicDashboardControl` dispatches rendering by `widgetType`:

| `widgetType` | Rendered output | Props passed |
|--------------|-----------------|--------------|
| `"static"` | Background image layer only | `control`, `frame` |
| `"text"` | Text span with formatted value | `control`, `frame`, `gearState?` |
| `"chart"` | `<ChartWidget>` component | `control`, `history` |
| `"map"` | `<MapWidget>` component | `control`, `frame`, `trackPoints` |

Each branch receives the control's layout rectangle (`x`, `y`, `width`, `height`) for positioning.

### 6. Data Injection Principle

All data consumed by the local dashboard overlay is injected through React props. Rendering components are **controlled** and do **not** call Tauri `invoke`.

Data flow:

- `frame` — injected by the main module's telemetry polling loop
- `history` — injected by the main module's chart history buffer
- `trackPoints` — injected by the main module's track geometry cache

This keeps the overlay module stateless and makes it easy to test in isolation.

### 7. Shared Type Source

All TypeScript types used by this contract are imported from `module_dashboard_protocol/types`:

```ts
import {
  DashboardControl,
  DashboardLayoutPayload,
  DashboardValuesFrame,
  WidgetType,
  ChartFieldConfig,
  DashboardConditionalRule,
  normalizeLayoutPayload,
} from "module_dashboard_protocol/types";
```

Do not duplicate these definitions inside `module_local_dashboard`. If a type is missing, extend `module_dashboard_protocol` first.

### 8. Backward Compatibility

Layouts stored with the old shape are automatically upgraded at load time.

**Mapping rules:**

- `isDynamic: true` → `widgetType: "text"`
- `isDynamic: false` → `widgetType: "static"`
- `staticControls` and `dynamicControls` → merged into a single `controls` array

Use `normalizeLayoutPayload()` from `module_dashboard_protocol` when loading legacy layouts:

```ts
const payload = normalizeLayoutPayload(rawJson);
```

After normalization, `payload.controls` contains only `DashboardControl` objects with a valid `widgetType`.

