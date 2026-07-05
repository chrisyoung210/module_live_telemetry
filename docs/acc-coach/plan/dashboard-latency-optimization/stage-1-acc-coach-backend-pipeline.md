# 阶段 1：acc-coach 后端链路优化

日期：2026-06-29
涉及模块：`acc-coach`（仅此模块）
依赖：无（`module_live_telemetry` 已提供所需 public API）

---

## 1. 目标

消除后端数据链路中的积压风险和不必要延迟，将后端转发延迟从 0~533ms（积压）降到 0~1ms。

## 2. 当前问题

### 2.1 bounded(64) 通道积压

`src/recording/auto.rs:435`：

```rust
let (dashboard_tx, dashboard_rx) = bounded::<DashboardValuesFrame>(64);
```

使用 `RecordingController::start(..., Some(dashboard_tx), ...)` → `DashboardOutput::Legacy` → `ChannelSink`。

`ChannelSink::send` (`src/dashboard/sink.rs:357`) 用 `try_send`，通道满时丢新帧。但 64 帧容量在 120Hz 下可缓存 533ms 旧帧，consumer 如果处理慢，会持续消费旧帧。

### 2.2 录制线程 1ms sleep

`src/recording/auto.rs:634`：

```rust
Err(crossbeam_channel::TryRecvError::Empty) => {
    std::thread::sleep(Duration::from_millis(1));
}
```

通道空时 sleep(1ms)，引入 0~1ms 不必要延迟，且 CPU 有空转空间。

### 2.3 drain 循环中每帧做 bus push

`src/recording/auto.rs:614-629`：drain 循环对每一帧都执行 `push_merged_to_bus`，但 `DashboardFrameBus` 只保留最新帧，中间帧的 bus push 全部被覆写。

`push_merged_to_bus` (`src/recording/auto.rs:1070`) 每次执行：
- 分配新 `HashMap<String, f64>`
- 逐 key 做 `alias.to_user_facing` 翻译
- `bus.push_frame`（内部 clone frame）

120Hz 下每秒 120 次 HashMap 分配 + alias 翻译，但 IPC 只有 ~30Hz 读取。

### 2.4 alias 翻译在录制线程

`push_merged_to_bus` 在 120Hz 录制线程做 alias 翻译，但翻译结果只有 IPC 30Hz 轮询时才被读取。翻译应在消费侧（IPC 线程）做，而非生产侧（录制线程）。

### 2.5 多次 clone

drain 循环中每帧有 2~3 次 `frame.clone()`：
- `telemetry_tx.try_send(frame.clone())` — 远程转发
- `merge_latest_dashboard_frame(frame.clone())` — 合并缓存
- `push_merged_to_bus` 内部 `bus.push_frame(&frame)` — bus API 强制 clone

## 3. 改动方案

### 3.1 A1：切换 LatestValueSink

**改动文件**：`src/recording/auto.rs`

将 `RecordingController::start` + `bounded(64)` 替换为 `RecordingController::start_with_latest_dashboard`。

`module_live_telemetry` 已提供 public API：

```rust
// src/recording/controller.rs:115
pub fn start_with_latest_dashboard(
    request: RecordingRequest,
    status_tx: StatusSender,
    outcome_tx: OutcomeSender,
    lap_completed: Option<LapCompletedCallback>,
) -> TelemetryResult<(Self, LatestValueReceiver)>
```

此 API 内部使用 `LatestValueSink`（`src/dashboard/sink.rs:309`），容量 1，覆写语义：
- producer 写入时如果 slot 已有帧，直接覆写（不积压）
- `send_latest` (`src/dashboard/sink.rs:109`) 自动合并同 generation 的稀疏帧（`pending.values.extend(frame.values)`）
- 内置 subscription_generation 处理（stale 帧自动丢弃）

**具体改动**：

```rust
// 替换 auto.rs:435 的
let (dashboard_tx, dashboard_rx) = bounded::<DashboardValuesFrame>(64);

// 和 auto.rs:470 的
let mut ctrl = match RecordingController::start(
    request, status_tx, outcome_tx, Some(dashboard_tx), Some(lap_completed),
) { ... };

// 改为
let (mut ctrl, dashboard_rx) = match RecordingController::start_with_latest_dashboard(
    request, status_tx, outcome_tx, Some(lap_completed),
) { ... };
```

`LatestValueReceiver` 提供：
- `try_recv()` — 非阻塞读取最新帧（取出并清空 slot）
- `recv_timeout(timeout)` — 阻塞等待指定时间
- `notification_receiver()` — 返回 `crossbeam_channel::Receiver<()>`，有新帧时收到 `()` 通知

### 3.2 A2：消除 1ms sleep，改为 Select 阻塞等待

**改动文件**：`src/recording/auto.rs`

用 crossbeam `Select` 同时监听 `dashboard_rx.notification_receiver()` 和 `command_rx`，替代 `try_recv` + `sleep(1ms)` 轮询。

**具体改动**（`auto_recording_loop` 内层循环 `'inner`）：

```rust
use crossbeam_channel::Select;

// 在 RecordingController 启动后，获取 notification receiver
let dashboard_notify = dashboard_rx.notification_receiver();

'inner: loop {
    // 1. 先 drain outcome/status（保持原有优先级逻辑不变）
    // ... outcome_rx try_recv loop ...
    // ... status_rx try_recv loop ...

    // 2. drain command（保持原有逻辑不变）
    // ... command_rx try_recv ...

    // 3. 等待 dashboard 帧或命令（阻塞，不 sleep）
    let mut sel = Select::new();
    let dash_oper = sel.recv(&dashboard_notify);
    let cmd_oper = sel.recv(&command_rx);
    let oper = sel.select();
    match oper.index() {
        i if i == cmd_oper => {
            match oper.recv(&command_rx) {
                Ok(AutoRecordingCommand::Stop) => { /* 原有处理 */ }
                Ok(AutoRecordingCommand::ReplaceDashboardItems { .. }) => { /* 原有处理 */ }
                Ok(AutoRecordingCommand::UpdateRemoteSubscriptions { .. }) => { /* 原有处理 */ }
                Err(_) => { /* 原有处理 */ }
            }
        }
        i if i == dash_oper => {
            // 收到通知，drain notification 并读取最新帧
            let _ = oper.recv(&dashboard_notify);
            // drain dashboard frames（A3 优化后的逻辑）
            drain_dashboard_frames(&dashboard_rx, &latest_dashboard_frame, &bus, &telemetry_tx, &alias, &mut dashboard_rx_log);
        }
        _ => {}
    }
}
```

**注意**：`notification_receiver()` 每次 `send_latest` 都会 `notify_tx.try_send(())`，但 `notify_rx` 容量 1，多次通知合并为一个。drain 时需要循环 `try_recv` 直到 Empty。

### 3.3 A3：drain 循环中只处理最终帧的 bus push

**改动文件**：`src/recording/auto.rs`

当前 drain 循环对每帧都做 `merge_latest_dashboard_frame` + `push_merged_to_bus`。优化为：
- drain 循环中只做 `merge_latest_dashboard_frame`（维护缓存）
- 循环结束后做**一次** `push_merged_to_bus`

**具体改动**：

```rust
fn drain_dashboard_frames(
    dashboard_rx: &LatestValueReceiver,
    latest_dashboard_frame: &Arc<Mutex<Option<AutoDashboardFrame>>>,
    bus: &DashboardFrameBus,
    telemetry_tx: &Option<TelemetryForwarderSender>,
    alias: &crate::dashboard::alias::ChannelAliasTable,
    log: &mut DashboardFrameLogSampler,
) {
    let mut frame_count = 0;
    loop {
        match dashboard_rx.try_recv() {
            Ok(frame) => {
                log_dashboard_queue_frame(log, &frame, 0); // backlog 始终 0（LatestValueSink）
                if let Some(ref tx) = telemetry_tx {
                    let _ = tx.try_send(AutoDashboardFrame::from(frame.clone()));
                }
                merge_latest_dashboard_frame(latest_dashboard_frame, frame);
                frame_count += 1;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => break,
            Err(crossbeam_channel::TryRecvError::Disconnected) => break,
        }
    }
    // 只在 drain 完成后做一次 bus push
    if frame_count > 0 {
        push_merged_to_bus(latest_dashboard_frame, bus, alias);
    }
}
```

**注意**：`telemetry_tx`（远程转发）仍需要对每帧做 `try_send`，因为远程路径需要尽可能多的帧。如果远程也需要最新值语义，后续阶段6会改为 CompactPatch。当前阶段保持原有行为。

### 3.4 A4：alias 翻译从录制线程移到 IPC 线程

**改动文件**：`src/recording/auto.rs`、`src/ipc/mod.rs`

#### auto.rs 变更

`push_merged_to_bus` 不再做 alias 翻译，直接推送 raw key frame 到 bus：

```rust
fn push_merged_to_bus(
    latest_dashboard_frame: &Arc<Mutex<Option<AutoDashboardFrame>>>,
    bus: &DashboardFrameBus,
    _alias: &crate::dashboard::alias::ChannelAliasTable,  // 不再使用
) {
    if let Some(frame) = latest_dashboard_frame
        .lock()
        .ok()
        .as_ref()
        .and_then(|f| f.as_ref())
    {
        // 直接推送 raw key，不翻译
        bus.push_frame(&module_dashboard_protocol::DashboardValuesFrame {
            sample_tick: frame.sample_tick,
            timestamp_ns: frame.timestamp_ns,
            values: frame.values.clone(),  // raw key，直接 clone
        });
    }
}
```

#### ipc/mod.rs 变更

`poll_dashboard_frame` 的 live 路径添加 alias 翻译（与 replay 路径一致）：

```rust
#[tauri::command]
async fn poll_dashboard_frame(
    bus: tauri::State<'_, Arc<DashboardFrameBus>>,
    replay: tauri::State<'_, ReplayStateType>,
    alias: tauri::State<'_, ChannelAliasTable>,
) -> IpcResult<Option<LiveDashboardFrame>> {
    if let Some(frame) = replay.latest_dashboard_frame() {
        let mut result = LiveDashboardFrame::from(frame);
        translate_dashboard_frame_values(&mut result.values, &alias);
        bus.push_frame(&module_dashboard_protocol::DashboardValuesFrame {
            sample_tick: result.sample_tick,
            timestamp_ns: result.timestamp_ns,
            values: result.values.clone(),
        });
        return Ok(Some(result));
    }
    let Some(frame) = bus.latest_frame() else {
        return Ok(None);
    };
    let mut result = LiveDashboardFrame {
        subscription_generation: 0,
        sample_tick: frame.sample_tick,
        timestamp_ns: frame.timestamp_ns,
        values: frame.values,  // raw key
    };
    translate_dashboard_frame_values(&mut result.values, &alias);  // 翻译为 user-facing
    log_live_dashboard_ipc_frame(&result);
    Ok(Some(result))
}
```

同理，`get_live_dashboard_frame` 的 live 路径也添加翻译。

**净效果**：alias 翻译从 120Hz 降到 ~30Hz（IPC 轮询频率），且录制线程每帧少一次 HashMap 分配 + 逐 key 翻译。

### 3.5 A5：减少 clone

**改动文件**：`src/recording/auto.rs`

`LatestValueReceiver::try_recv` 返回 `DashboardValuesFrame`（所有权转移），不需要 clone。

`merge_latest_dashboard_frame` 当前接收 `DashboardValuesFrame`（by value），内部 clone 到缓存。优化为 move values 到缓存，避免 clone。

`telemetry_tx` 仅在远程 dashboard 激活时存在，可条件 clone：

```rust
// drain 循环中
match dashboard_rx.try_recv() {
    Ok(frame) => {
        if let Some(ref tx) = telemetry_tx {
            let _ = tx.try_send(AutoDashboardFrame::from(frame.clone()));
        }
        // move frame 到 merge（不 clone）
        merge_latest_dashboard_frame(latest_dashboard_frame, frame);
    }
    // ...
}
```

如果 `telemetry_tx` 不存在（远程未激活），frame 直接 move 到 merge，零 clone。

## 4. 模块间开发顺序

本阶段仅涉及 `acc-coach`，无跨模块协调。改动集中在 `src/recording/auto.rs` 和 `src/ipc/mod.rs`。

建议开发顺序：
1. A1（LatestValueSink 切换）— 最核心，改变通道类型
2. A2（Select 阻塞）— 依赖 A1 的 `notification_receiver`
3. A3（drain 优化）— 在 A1/A2 基础上调整 drain 逻辑
4. A4（alias 移位）— 独立于 A1-A3，可并行
5. A5（clone 减少）— 在 A1 基础上微调

## 5. 验收标准

| 验收项 | 验证方法 |
|---|---|
| overlay 正常显示速度/转速/档位等 | 启动 ACC + recording，观察 overlay |
| 无 "--" 闪烁 | 驾驶一段，观察不同刷新率字段 |
| 后端无积压 | 设置 `ACC_DASHBOARD_LOG=1`，观察 `queuedAfterRecv` 始终为 0 |
| CPU 不上升 | 任务管理器对比改动前后 |
| 暂停/恢复正常 | 暂停 ACC 再恢复，overlay 状态正确 |
| 退出 session | 退出后 overlay 隐藏，snapshot 清空 |
| remote dashboard 不受影响 | 如有 remote 设备，验证帧流正常 |

## 6. 风险

| 风险 | 缓解 |
|---|---|
| `LatestValueReceiver` API 行为与 `ChannelSink` 不同 | 已有单元测试覆盖（`src/dashboard/sink.rs` tests），行为是覆写语义 |
| `Select` 阻塞可能错过 outcome/status | 保持原有优先级 drain 逻辑（先 drain outcome/status 再 Select） |
| alias 翻译移到 IPC 后，replay 路径已翻译但 live 路径 bus 存 raw key | `poll_dashboard_frame` 中 live 路径添加翻译，与 replay 路径一致 |
| `telemetry_tx` 仍用 `AutoDashboardFrame::from(frame.clone())` | 保持原有行为，阶段6改 remote 路径时优化 |

## 7. 参照文档

- `docs/acc-coach/live-telemetry-api-boundary.md` — module_live_telemetry API 边界
- `docs/acc-coach/2026-06-16-dashboard-display-performance-plan.md` — 早期性能计划（问题2节分析了通道积压）
- README.md 关键代码位置索引 — module_live_telemetry `LatestValueSender/Receiver` 条目
