# Local Dashboard 热更新方案

## 背景

当前修改 layout 或 local dashboard 控件后，preview mode 和 replay mode 都无法热响应，必须重启软件才能生效。

## 架构分析

覆盖层窗口（`local-dashboard-overlay`）是一个**独立的 Tauri webview 窗口**，与主窗口（`main`）是两个独立的 webview，通过 Tauri IPC 与 Rust 后端通信。

```
┌─────────────────────────────┐    ┌──────────────────────────────┐
│  主窗口 (main)               │    │  覆盖层窗口 (local-dashboard-  │
│                             │    │  overlay)                    │
│  DashboardDesignerView      │    │                              │
│  └─ register_dashboard_     │    │  LocalDashboardOverlayWindow  │
│     layout IPC ─────────────┼───→│  └─ LocalDashboardOverlay     │
│                             │    │     └─ useDashboardMetadata() │
│  LocalDashboardView         │    │        ├─ get_local_dashboard │
│  └─ saveLocalDashboard      │    │        │  _overlay_config IPC │
│     OverlayConfig IPC ──────┼───→│        └─ list_registered_    │
│                             │    │           dashboard_layouts   │
│                             │    │           IPC                 │
│                             │    │     └─ useDashboardFrame()    │
│                             │    │        └─ poll_dashboard_     │
│                             │    │           frame IPC           │
└─────────────────────────────┘    └──────────────────────────────┘
```

### 关键发现

1. **`LocalDashboardOverlay` 是自给自足的**：不接收 layout 数据作为 props，而是通过 `useDashboardMetadata()` hook 在 mount 时自行调用 `list_registered_dashboard_layouts` IPC 从磁盘读取布局。

2. **覆盖层窗口与主窗口共享同源 localStorage**：两个窗口加载同一 URL（仅 query param 不同），`storage` 事件天然支持跨窗口通知——代码库已有先例（`accCoachDashboardPreview` 键）。

3. **Replay 订阅在启动时固定**：`ReplayState::start()` 一次性读取布局计算订阅，运行时无法更新。

4. **跨 webview 通知机制统一为 `localStorage`**：本方案不再依赖 Rust 后端事件广播，而是在主窗口保存成功后写入同源 `localStorage`，由已启动的覆盖层窗口通过 `storage` 事件接收通知。`app.emit()` 相关逻辑需要移除，避免同时维护两套通知机制。

5. **未启动覆盖层不需要实时通知**：如果覆盖层窗口尚未启动，保存后的 layout/config 已写入磁盘；覆盖层后续启动时会通过 IPC 从磁盘读取最新数据。`localStorage` 通知只用于刷新已经打开并已挂载的覆盖层窗口。

## 方案设计

### 核心思路

1. **Preview Mode**：利用 `localStorage` + `storage` 事件实现已打开覆盖层窗口的跨窗口通知，通过重新加载 overlay config + React `key` prop 强制 `LocalDashboardOverlay` 重新挂载，触发 `useDashboardMetadata()` 重新读取布局。

2. **Replay Mode**：在 `ReplayState` 上新增 `replace_dashboard_items()` 方法，布局保存时同步更新重放管道的遥测订阅。

3. **冷启动场景**：如果覆盖层窗口尚未启动，不需要热通知；layout 或 local dashboard overlay config 保存后已经落盘，后续打开覆盖层时会读取最新磁盘状态。

### Preview Mode 热更新流程

```
DashboardDesignerView (主窗口) 保存布局
  → register_dashboard_layout IPC 成功
  → localStorage.setItem("accCoachDashboardLayoutsVersion", uniqueVersion())
  ↓ (仅已打开的覆盖层窗口会收到 storage 事件)
LocalDashboardOverlayWindow (覆盖层窗口)
  → window.addEventListener("storage", ...) 检测到 key 变化
  → loadConfig() 重新读取 local_dashboard_overlay.json
  → setLayoutVersion(v => v + 1)
  → <LocalDashboardOverlay key={layoutVersion} /> 强制卸载+重新挂载
  → useDashboardMetadata() mount 时重新调用 list_registered_dashboard_layouts IPC
  → 渲染新布局和最新 overlay config

覆盖层窗口未启动时：不需要 storage 事件；下次启动时直接从磁盘读取最新 layout/config。
```

### Replay Mode 热更新流程

```
DashboardDesignerView (主窗口) 保存布局
  → register_dashboard_layout IPC 成功
  → sync_replay_subscriptions_from_disk(&app, &replay)
    → 从磁盘重读 local_dashboard_overlay.json + dashboard_layouts.json
    → 计算新的 DashboardItemSubscription 列表（当前会复用 dashboard_subscriptions_from_app_data，因此包含 remote dashboard 订阅和 local dashboard 订阅）
    → replay.replace_dashboard_items(items)
      → ctrl.replace_dashboard_items(&items)  // 转发给 RecordingController
  → 重放帧数据立即包含新布局的遥测字段
```

## 修改清单

### 1. Rust 后端

#### `src/recording/replay.rs` — 新增 `replace_dashboard_items` 方法 ✅ 已完成

```rust
/// Replace dashboard subscriptions on the active replay controller.
/// No-op if no replay is active.
pub fn replace_dashboard_items(
    &self,
    items: Vec<DashboardItemSubscription>,
) -> Result<(), String> {
    let inner = self.inner.lock().map_err(|e| format!("Replay lock: {e}"))?;
    let Some(ref ctrl) = inner.controller else {
        return Ok(());
    };
    ctrl.replace_dashboard_items(&items)
        .map_err(|e| format!("Failed to replace replay dashboard items: {e:?}"))
}
```

#### `src/ipc/mod.rs` — 修改

**a) 去掉 `Emitter` import 和 `app.emit()` 调用**

当前代码引入了 `use tauri::{Emitter, Manager}` 并在 3 个 IPC 命令中调用了 `app.emit("dashboard-layouts-changed", ())`。需要**移除**：
- `Emitter` import → 恢复为 `use tauri::Manager;`
- 3 处 `let _ = app.emit("dashboard-layouts-changed", ());`

**b) 新增 `sync_replay_subscriptions_from_disk` 辅助函数** ✅ 已完成

```rust
fn sync_replay_subscriptions_from_disk(
    app: &tauri::AppHandle,
    replay: &ReplayState,
) -> IpcResult<()> {
    if !replay.is_active() {
        return Ok(());
    }
    let app_data_dir = app.path().app_data_dir()...;
    let (items, _remote_items) = dashboard_subscriptions_from_app_data(&app_data_dir, app)?;
    replay.replace_dashboard_items(items).map_err(AppError::from)?;
    Ok(())
}
```

**c) 3 个 IPC 命令增加 `replay` 参数** ✅ 已完成

- `save_local_dashboard_overlay_config` — 增加 `replay: tauri::State<'_, ReplayStateType>`
- `register_dashboard_layout` — 增加 `replay: tauri::State<'_, ReplayStateType>`
- `delete_registered_dashboard_layout` — 增加 `replay: tauri::State<'_, ReplayStateType>`

每个命令末尾调用 `sync_replay_subscriptions_from_disk(&app, &replay)?;`

### 2. 前端 TypeScript

#### 版本通知值生成

为了避免同一毫秒内连续保存导致 `storage` 新旧值相同，使用唯一版本值：

```typescript
function nextDashboardLayoutsVersion() {
  return `${Date.now()}:${crypto.randomUUID()}`;
}
```

#### `src-ui/components/DashboardDesignerView.tsx` — 保存布局后写 localStorage

在 `register_dashboard_layout` IPC 成功后（第 2094 行附近），添加：

```typescript
// 通知覆盖层窗口布局已变更
window.localStorage.setItem(
  "accCoachDashboardLayoutsVersion",
  nextDashboardLayoutsVersion(),
);
```

#### `src-ui/components/LocalDashboardView.tsx` — 保存 overlay 配置后写 localStorage

在 `saveOverlay()` 函数中 `saveLocalDashboardOverlayConfig` 成功后（第 402 行附近），添加：

```typescript
window.localStorage.setItem(
  "accCoachDashboardLayoutsVersion",
  nextDashboardLayoutsVersion(),
);
```

#### `src-ui/components/LocalDashboardOverlayWindow.tsx` — 监听 storage 事件

**a) 移除 `listen` import 和 Tauri 事件监听**

当前代码引入了 `import { listen } from "@tauri-apps/api/event"` 和对应的 `useEffect` 监听 `dashboard-layouts-changed`。需要**移除**这些。

**b) 在已有的 `storage` 事件监听中增加 layout 版本检测**

当前代码已有 `window.addEventListener("storage", syncPreviewMode)` 监听（第 157 行）。需要**扩展**这个监听器，同时检测 `accCoachDashboardLayoutsVersion` 键：

```typescript
useEffect(() => {
    const syncPreviewMode = () => {
      const enabled = dashboardPreviewEnabled();
      setPreviewMode(enabled);
      if (enabled) {
        loadConfig();
      }
    };
    const onStorage = (e: StorageEvent) => {
      if (e.key === DASHBOARD_PREVIEW_STORAGE_KEY) {
        syncPreviewMode();
      }
      if (e.key === "accCoachDashboardLayoutsVersion") {
        loadConfig();
        setLayoutVersion((v) => v + 1);
      }
    };
    const reloadOnFocus = () => {
      loadConfig();
    };
    window.addEventListener("storage", onStorage);
    window.addEventListener("focus", reloadOnFocus);
    return () => {
      window.removeEventListener("storage", onStorage);
      window.removeEventListener("focus", reloadOnFocus);
    };
  }, []);
```

**c) `layoutVersion` 作为 `LocalDashboardOverlay` 的 `key`** ✅ 已完成

```tsx
<LocalDashboardOverlay
  key={layoutVersion}
  visible={visible}
  ...
/>
```

**d) 已启动与未启动行为边界**

- 覆盖层窗口已启动：依赖 `storage` 事件触发 `loadConfig()` 和 `layoutVersion` 增量，实现热更新。
- 覆盖层窗口未启动：不会接收 `storage` 事件，但后续启动时会执行初始 `loadConfig()`，并由 `LocalDashboardOverlay` 自行读取最新注册布局。
- 未保存的编辑不会触发热更新；本方案以保存成功后的磁盘状态为准。

## 不修改的模块

- `module_local_dashboard` — 禁止修改
- `module_live_telemetry` — 禁止修改
- `acctlm_core` — 禁止修改
- `ld_to_acctlm` — 禁止修改

## 测试验证

1. **Preview Mode 热更新**：
   - 打开 Dashboard Designer，修改布局（添加/删除/移动控件），保存
   - 点击 Preview 打开预览
   - 回到 Designer，再次修改布局并保存
   - 预览应立即反映新布局（无需关闭重开）

2. **Replay Mode 热更新**：
   - 启动 replay
   - 修改布局并保存
   - replay 帧数据应立即包含新布局的遥测字段

3. **不修改布局时无副作用**：
   - 正常使用 preview/replay，不触发不必要的重新挂载
