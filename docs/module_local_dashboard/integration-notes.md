# Local Dashboard Overlay Integration Notes

This directory is a code-level submodule, not a standalone Tauri app.

## Rust workspace integration

Add this crate as a workspace member or path dependency from the main module:

```toml
acc-coach-local-dashboard-overlay = { path = "../module_local_dashboard" }
```

In the main Tauri setup:

```rust
acc_coach_local_dashboard_overlay::local_dashboard_overlay::setup(app)?;
```

The main module owns IPC, live-session state, polling, visibility decisions, and
configuration workflows. This module exposes data types and helpers; it does not
require the main module to implement a fixed host command contract.

```rust
use acc_coach_local_dashboard_overlay::local_dashboard_overlay::{
    acc_window::find_acc_window_bounds,
    config::LocalDashboardOverlayConfig,
    window::{
        hide_overlay_window,
        set_overlay_bounds,
        set_overlay_click_through,
        show_overlay_window,
    },
};
```

## Frontend integration

Import feature code from:

```text
src-ui/features/local-dashboard-overlay
```

The host app keeps its existing React entry and state management. It passes
fully controlled props into `LocalDashboardOverlay`:

```tsx
<LocalDashboardOverlay
  config={config}
  containerWidth={accWindowWidth}
  containerHeight={accWindowHeight}
  frame={liveFrame}
  layouts={registeredLayouts}
  visible={overlayVisible}
/>
```

`OverlayRegionEditor` and `OverlayRegionPreview` can be used inside the host
module's existing `LocalDashboardView`.
