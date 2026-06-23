# module_live_telemetry API 边界需求

- 日期：2026-06-14
- 目标：`acc-coach` 不直接依赖 ACC shared memory、ACC 游戏状态解析、ACC 窗口识别等底层细节；所有和 ACC 游戏直接相关的能力由 `module_live_telemetry` 提供稳定 API。
- 核心原则：**dashboard 所需数据必须来自 `RecordingController` 的 dashboard item 订阅输出，或运行时动态新增的 dashboard item 订阅。**

## 结论

当前代码已经有一个接近目标的 live runtime：`module_live_telemetry::recording::RecordingController`。

因此不应优先新增一套并行的 `LiveTelemetryRuntime`。更合理的方向是：

- 继续使用 `RecordingController` 作为唯一 ACC live 数据源。
- 使用 `status_tx` 提供 ACC 连接、live、paused、recording 等状态。
- 使用 `dashboard_tx` 提供所有 dashboard 动态值。
- 使用 raw item catalog、calculated item registry，以及运行时动态注册的 calculated item 作为 dashboard 可订阅项来源。
- 使用 `add_dashboard_item`、`remove_dashboard_item`、`replace_dashboard_items` 支持运行时动态订阅。
- `acc-coach` 只缓存和转发 `RecordingController` 输出，不直接读 shared memory。

## 当前已满足的 module API

根据 `docs/module_live_telemetry/api/recording_additional_0614.md`，以下接口已经由 `module_live_telemetry` 提供或明确不需要提供。

### 1. DashboardValuesFrame 输出

`RecordingController::start` 的 `dashboard_tx` 已经输出 `DashboardValuesFrame`：

```rust
use crossbeam_channel::Sender;
use module_live_telemetry::recording::DashboardValuesFrame;

impl RecordingController {
    pub fn start(
        request: RecordingRequest,
        status_tx: StatusSender,
        outcome_tx: OutcomeSender,
        dashboard_tx: Option<Sender<DashboardValuesFrame>>,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<RecordingController>;
}
```

```rust
use std::collections::HashMap;

pub struct DashboardValuesFrame {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub values: HashMap<String, f64>,
}
```

约定：

- `values` 目前按 `HashMap<String, f64>` 处理。
- 未来 dashboard value type 可能会扩展，但不是当前阶段目标。
- `acc-coach` 侧应从旧的 `HashMap<String, f64>` 缓存迁移到 `DashboardValuesFrame` 缓存。

### 2. RecordingStatus 事件流

`status_tx` 输出 `RecordingStatus`。目前不要求 `module_live_telemetry` 新增 `RecordingStatusSnapshot` 或 `reduce_recording_status`。

当前设计保持：

- `module_live_telemetry` 继续输出事件流。
- `acc-coach` 的 `AutoRecordingMonitor` 继续自行归约为 app 层状态。
- app 层状态继续使用当前的 `AutoRecordingStatus`。

这样能维持现状，也避免把 UI 状态机提前下沉到 `module_live_telemetry`。

### 3. Dashboard subscription validation

`module_live_telemetry` 已提供订阅校验：

```rust
pub struct DashboardSubscriptionValidation {
    pub valid: bool,
    pub errors: Vec<DashboardSubscriptionError>,
}

pub struct DashboardSubscriptionError {
    pub item_name: String,
    pub message: String,
}

pub fn validate_dashboard_subscriptions(
    items: &[DashboardItemSubscription],
) -> DashboardSubscriptionValidation;
```

用途：

- layout 保存前校验动态控件引用的 item。
- remote/serial output profile 保存前校验 channel。
- `replace_dashboard_items` 前提前拒绝无效订阅。

### 4. replace_dashboard_items 明确结果

`RecordingController::replace_dashboard_items` 已经合并为明确返回结果：

```rust
impl RecordingController {
    pub fn replace_dashboard_items(
        &self,
        items: &[DashboardItemSubscription],
    ) -> Result<(), DashboardSubscriptionError>;
}
```

成功语义是：

- 订阅项校验成功。
- replace command 已成功发送或入队。

非语义：

- 不表示 dashboard service 已同步完成替换。
- 不表示下一帧一定已经包含新订阅项。

因此当前不需要新增：

```rust
RecordingStatus::DashboardSubscriptionError {
    item_name: String,
    message: String,
}
```

订阅错误由 `replace_dashboard_items` 的同步 `Result` 返回；`RecordingStatus` 继续表示 recording lifecycle / shared memory / recording error 等状态。

### 5. 当前订阅项查询

`RecordingController::list_dashboard_items(&self) -> Vec<DashboardItemInfo>` 已提供，但它的语义是查询当前 `RecordingController` 维护的 dashboard 订阅项，不是查询全部可订阅 catalog。

如果 API 名称未来还可以调整，更清晰的命名会是：

```rust
pub fn list_dashboard_subscriptions(&self) -> Vec<DashboardItemInfo>;
```

如果暂时不改名，`acc-coach` 侧文档和调用处必须按“当前订阅项”理解它，不能把它当成全部 raw/calculated item catalog。

## Dashboard catalog 来源

不需要新增一个单独的“全部 dashboard item catalog”接口。

原则上：

- 所有 raw item 都必须可用于 dashboard item。
- 所有 calculated item 都必须可用于 dashboard item。
- 运行时调用方可以用表达式动态注册 calculated item。
- 动态 calculated item 一旦注册成功，就自动成为可订阅的 dashboard item。

因此 dashboard designer / output profile 的可选项来源应复用现有 raw item 与 calculated item 获取能力，而不是要求 `module_live_telemetry` 再提供一套重复 catalog。

需要注意的边界：

- `validate_dashboard_subscriptions` 必须能识别已存在的 raw item。
- `validate_dashboard_subscriptions` 必须能识别已注册的 calculated item。
- 如果 calculated item 是运行时由调用方注册的，校验必须使用同一个 registry / controller / dashboard service 上下文，避免默认 registry 看不到动态项。

## acc-coach 侧当前缺口

### 1. AutoRecordingMonitor 需要启用 dashboard_tx

`AutoRecordingMonitor` 启动 controller 时应传入 dashboard channel：

```rust
let (dashboard_tx, dashboard_rx) = bounded::<DashboardValuesFrame>(N);

RecordingController::start(
    request,
    status_tx,
    outcome_tx,
    Some(dashboard_tx),
    None,
)
```

`AutoRecordingMonitor` 需要消费 `dashboard_rx` 并缓存最新 `DashboardValuesFrame`。

### 2. AutoRecordingMonitor 需要提供 app 内部 dashboard API

建议 app 侧提供：

```rust
impl AutoRecordingMonitor {
    pub fn status(&self) -> AutoRecordingStatus;
    pub fn latest_dashboard_frame(&self) -> Option<DashboardValuesFrame>;
    pub fn replace_dashboard_items(
        &self,
        items: &[DashboardItemSubscription],
    ) -> Result<(), DashboardSubscriptionError>;
}
```

职责：

- `status()` 返回最新 app 层状态。
- `latest_dashboard_frame()` 返回最新 dashboard item values 与时间戳。
- `replace_dashboard_items()` 将 layout / output profile 需要的动态字段同步给 `RecordingController`。

### 3. get_live_dashboard_frame 需要迁移

当前 `acc-coach::ipc::get_live_dashboard_frame` 直接使用：

```rust
module_live_telemetry::shmem::AccSharedMemoryReader
```

这不符合边界设计，也会导致多处重复读取 ACC shared memory。

目标：

- 删除 `acc-coach` 对 `AccSharedMemoryReader` 的直接使用。
- 将前端 overlay 从“请求完整 LiveFrame”改为“请求 dashboard frame / dashboard values”。
- 只有 `module_live_telemetry` 内部可以读 shared memory。

建议 IPC：

```rust
#[tauri::command]
async fn get_live_dashboard_frame(
    monitor: tauri::State<'_, AutoRecordingMonitor>,
) -> IpcResult<Option<DashboardValuesFrame>> {
    Ok(monitor.latest_dashboard_frame())
}
```

如果前端仍需要状态和数值一起返回：

```rust
pub struct LiveDashboardSnapshot {
    pub status: AutoRecordingStatus,
    pub frame: Option<DashboardValuesFrame>,
}
```

### 4. Dashboard layout 到订阅项的转换需要统一

所有 dashboard layout 的动态控件都应被转换成 `DashboardItemSubscription`。

需要一个统一转换函数：

```rust
pub fn dashboard_subscriptions_for_layouts(
    layouts: &[DashboardLayoutPayload],
) -> Vec<DashboardItemSubscription>;
```

或以当前启用的 dashboard 输出为入口：

```rust
pub fn dashboard_subscriptions_for_active_dashboards(
    registered_layouts: &[RegisteredDashboardLayout],
    overlay_config: &LocalDashboardOverlayConfig,
    output_profiles: &OutputProfilesConfig,
) -> Vec<DashboardItemSubscription>;
```

要求：

- Local Dashboard、Remote Dashboard、Serial Dashboard 统一走同一套订阅计算。
- 去重相同 item。
- 根据 layout 动态控件、conditional rule、表达式依赖提取所需字段。
- layout 切换、region 变化、output profile 变化后调用 `replace_dashboard_items`。

## ACC 状态与 overlay 显示边界

ACC 是否启动、是否 live、是否 paused，必须来自 `RecordingController` 的 `RecordingStatus` 事件流，由 `acc-coach` 自行归约为 app 层状态。

不需要 `acc-coach` 直接调用 shared memory API 判断：

- ACC shared memory 是否存在。
- ACC 是否 live。
- ACC 是否 paused。

也不需要 `module_live_telemetry` 提供 ACC window bounds API。

Local Dashboard overlay 不再以 ACC 游戏窗口作为布局坐标系，而是以显示器范围作为绘制 layout 的范围。这样可以避免 `module_live_telemetry` 或 `module_local_dashboard` 编码 ACC 窗口标题、进程名、窗口查找规则，也避免 ACC 未启动时 overlay 坐标系不可用。

要求：

- `module_live_telemetry` 不提供 `find_acc_window_bounds`。
- `module_local_dashboard` 不负责查找 ACC 窗口。
- overlay 坐标系来自显示器 bounds / monitor bounds。
- ACC 未启动、未 live、paused 时是否显示 overlay 由 `RecordingController` 状态输出决定。

## 不建议新增的 API

在当前架构下，不建议优先新增以下并行 runtime：

```rust
LiveTelemetryRuntime
LiveTelemetryRuntimeConfig
LiveTelemetryRuntime::start
LiveTelemetryRuntime::status
LiveTelemetryRuntime::latest_frame
LiveTelemetryRuntime::subscribe_status
LiveTelemetryRuntime::subscribe_frames
LiveTelemetryRuntime::stop
```

原因：

- `RecordingController` 已经是 long-lived live runtime。
- `RecordingController` 已经负责打开 shared memory、reconnect、状态事件、recording loop、dashboard distribution。
- 新增并行 runtime 会带来双 shared memory reader、双状态机、双 frame stream 的风险。

只有当未来决定彻底重构 `RecordingController`，把 recording 变成 live runtime 的一个 consumer 时，才考虑引入该抽象。

也不需要新增：

```rust
pub fn find_acc_window_bounds() -> TelemetryResult<Option<AccWindowBounds>>;
```

原因：

- overlay layout 坐标系使用显示器范围。
- ACC 窗口查找不再是 dashboard 显示的前置条件。
- `module_live_telemetry` 只负责 ACC telemetry 和 recording/dashboard item 数据源。

## 目标数据流

```text
ACC shared memory
      |
module_live_telemetry::RecordingController
      |
      +-- status_tx -> AutoRecordingMonitor.status
      |
      +-- dashboard_tx -> AutoRecordingMonitor.latest_dashboard_frame
      |
      +-- outcome_tx -> recording persistence
      |
      +-- dynamic subscription commands
              add_dashboard_item
              remove_dashboard_item
              replace_dashboard_items

acc-coach IPC
      |
      +-- get_live_auto_recording_status
      +-- get_live_dashboard_frame / get_live_dashboard_snapshot
      +-- sync_dashboard_subscriptions

dashboards
      |
      +-- local overlay
      +-- remote UDP
      +-- serial / STM32
      +-- future web dashboard
```

## module_local_dashboard 侧边界

`module_local_dashboard` 应只负责：

- overlay window 创建、show/hide/close。
- click-through、always-on-top、透明窗口等窗口能力。
- overlay config。
- region/layout 渲染协议。
- 基于显示器 bounds / monitor bounds 设置绘制范围。

不负责：

- ACC shared memory。
- ACC live/paused 判定。
- dashboard item 计算。
- dashboard item 订阅。
- ACC window bounds 查找。

## 推荐迁移顺序

1. 修改 `AutoRecordingMonitor`，启用 `RecordingController::start(..., Some(dashboard_tx), ...)`。
2. 在 `AutoRecordingMonitor` 内消费 `dashboard_rx`，缓存 latest `DashboardValuesFrame`。
3. 增加 `get_live_dashboard_frame` 或 `get_live_dashboard_snapshot` IPC。
4. 修改 local overlay，不再调用直接读取 shared memory 的 `get_live_dashboard_frame`，改为读取 `RecordingController` 输出的 dashboard frame。
5. 将 layout / output profile 动态字段统一转换为 `DashboardItemSubscription`。
6. 在 layout / overlay / output profile 变化时调用 `replace_dashboard_items`，并按其 `Result` 处理校验或入队失败。
7. 删除 `acc-coach` 对 `AccSharedMemoryReader` 的直接使用。
8. 将 local overlay 坐标系改为显示器范围，不再依赖 ACC window bounds。
9. 收窄 `module_live_telemetry::shmem` 的 public surface，避免 app 层再次直接引用。

## 验收标准

- `acc-coach/src` 中不再出现 `AccSharedMemoryReader`。
- `acc-coach/src` 中不再直接 import `module_live_telemetry::shmem::*`。
- local dashboard、remote dashboard、serial dashboard 都消费 `RecordingController` 的 dashboard item 输出。
- dashboard 动态控件需要的字段都能被转换为 `DashboardItemSubscription`。
- 运行时动态注册的 calculated item 注册成功后可被 dashboard 订阅。
- layout 切换后 dashboard 订阅会更新。
- `replace_dashboard_items` 的成功含义按“校验成功 + replace command 成功发送/入队”处理。
- ACC 未启动、未 live、paused 时，overlay 保持隐藏。
- overlay layout 坐标系使用显示器范围，而不是 ACC window bounds。
- app 退出时 overlay window 被关闭，不只是隐藏。
