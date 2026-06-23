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

