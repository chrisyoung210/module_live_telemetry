# RecordingController API 参考

m2 暴露给 m1 的录制控制接口。m1 通过同步调用 + 独立 channel 模式启动/停止长期 ACC 遥测录制服务。一次 `RecordingController::start()` 调用会启动一个 holder 线程；该线程在软件生命周期内等待 ACC shared memory、录制每个 session，并对每个完成的 session 发送一个 `RecordingOutcome`。

> **相关文档**：[extract-telemetry.md](extract-telemetry.md) · [import.md](import.md) · [calculated-item.md](calculated-item.md) · [raw-item.md](raw-item.md) · [lap-completed-callback.md](lap-completed-callback.md)

---

## 1. 快速开始

```rust
use module_live_telemetry::recording::*;
use std::time::Duration;
use std::path::PathBuf;

fn main() -> TelemetryResult<()> {
    // 1. 构造请求
    let req = RecordingRequest {
        poll_hz: 60.0,                              // 30-120Hz
        output_dir: PathBuf::from("./recordings"),    // 必须已存在
        status_interval: Duration::from_secs(1),      // 状态回调间隔
        dashboard_items: vec![],
        dashboard_realtime_items: vec![],
    };

    // 2. 创建 channel（m1 侧接收端）
    let (status_tx, status_rx) = status_channel(16);  // 状态
    let (outcome_tx, outcome_rx) = outcome_channel(); // 每个 session 一个结果

    // 3. 启动长期录制服务（即使 ACC 尚未启动也会立即返回）
    let mut ctrl = RecordingController::start(
        req,
        status_tx, outcome_tx,
        None,  // 无 dashboard
        None,  // 无圈完成回调
    )?;

    // 4. 读取状态
    std::thread::spawn(move || {
        while let Ok(status) = status_rx.recv() {
            match status {
                RecordingStatus::Started { .. } => println!("录制服务已启动"),
                RecordingStatus::WaitingForSharedMemory { message } => println!("{message}"),
                RecordingStatus::Connected => println!("ACC 已连接"),
                RecordingStatus::RecordingStarted => println!("开始录制"),
                RecordingStatus::Running { sample_count, bytes_written, elapsed, fps } => {
                    // 当前 Running 会在 ACC 从 Pause 恢复到 Live 时立即发送一次。
                    println!("恢复录制: {sample_count} 帧, {fps:.1} Hz");
                }
                RecordingStatus::Paused => println!("ACC 已暂停，录制保持打开"),
                RecordingStatus::Stopping { reason } => {
                    println!("正在停止: {reason}");
                }
                RecordingStatus::Error { message, .. } => {
                    eprintln!("错误: {message}");
                }
                _ => {}
            }
        }
    });

    // 5. 获取结果（每完成一个 session 收到一次）
    while let Ok(outcome) = outcome_rx.recv() {
        println!("session 录制完成: {}", outcome.file_path.display());
        println!("  track: {}", outcome.track_name);
        println!("  car:   {}", outcome.car_model);
        println!("  type:  {}", outcome.session_type);
        println!("  date:  {}", outcome.recording_date);
        println!("  time:  {}", outcome.recording_time);
        println!("  laps:  {}", outcome.laps.len());
        println!("  size:  {} bytes", outcome.file_size_bytes);
    }

    Ok(())
}
```

---

## 2. 核心类型

### RecordingRequest

```rust
pub struct RecordingRequest {
    /// 轮询频率，范围 [30.0, 120.0] Hz
    pub poll_hz: f64,
    /// 输出目录（必须已存在，m2 自动命名文件）
    pub output_dir: PathBuf,
    /// 状态回调间隔
    pub status_interval: Duration,
    /// dashboard 订阅项（dashboard 基础设施始终初始化，items 驱动数据计算）
    pub dashboard_items: Vec<DashboardItemSubscription>,
    /// 启动时动态注册的 realtime calculated dashboard item
    pub dashboard_realtime_items: Vec<DashboardRealtimeItemRegistration>,
}
```

`validate()` 校验规则：
| 校验项 | 约束 |
|--------|------|
| `poll_hz` | `30.0 <= poll_hz <= 120.0`，必须有限（NaN/Inf 拒绝） |
| `output_dir` | 必须存在且为目录 |
| `status_interval` | 必须 > 0 |
| 文件名碰撞 | 自动追加 `-N` 后缀生成唯一文件名，不覆盖既有文件 |

### RecordingStatus

m2 → m1 的状态通知流：

```
Started → WaitingForSharedMemory* → Connected
Connected → RecordingStarted  ─── metadata 可查询 ✓
RecordingStarted/Running → Paused
Paused → Running (Pause → Live 恢复通知)
RecordingStarted/Running → Stopping(SessionEnd|ShmemLost)

手动 stop/drop:
Started/Waiting/Recording → Stopping(Manual) → holder 退出
```

回放状态流：

```
Started → ReplayStarted  ─── metadata 可查询 ✓
ReplayStarted → Running* → Stopping(FramesExhausted|Manual)
```

```rust
pub enum RecordingStatus {
    Started { thread_id: u64 },  // 后台线程已启动（但可能尚未连接 ACC）
    WaitingForSharedMemory {     // ACC shared memory 尚不可用；holder 会继续等待
        message: String,
    },
    Connected,                   // ACC 共享内存已连接
    RecordingStarted,            // writer 已创建，帧数据开始写入
    Running {                    // Pause → Live 后立即发送的恢复/进度事件
        sample_count: u64,       //   已录制帧数
        bytes_written: u64,      //   近似写入字节数
        elapsed: Duration,       //   录制已运行时长
        fps: f64,                //   实际录制频率
    },
    Paused,                      // ACC 暂停（writer 保持打开）
    Error { message: String, kind: RecordingErrorKind },
    Stopping { reason: StopReason },
}

pub enum StopReason {
    Manual,        // stop_recording() 手动调用
    SessionEnd,    // ACC session 结束（Live→Off）
    ShmemLost,     // 共享内存连接丢失
}

pub enum RecordingErrorKind {
    DiskFull,
    ShmemDisconnected,
    Unknown,
}
```

**状态事件语义**（供 m1 自行归约）：
- `WaitingForSharedMemory`：可视为未连接。
- `Connected`：可视为已连接，但不代表正在录制。
- `RecordingStarted`：可视为正在录制。
- `Paused`：ACC 从 Live 进入 Pause 时发送；writer 保持打开、采样暂停。调用方应设置 `paused = true`，并隐藏仅在 live 状态显示的 overlay。
- `Running`：已开始录制的 session 从 Pause 恢复到 Live 时立即发送一次。调用方应设置 `recording = true`、`paused = false`，并可恢复 live overlay。首次进入 Live 仍发送 `RecordingStarted`，不会额外发送 `Running`。
- `Stopping`：当前 recording/session 正在停止。
- `Error`：记录 `last_error`，是否清理其它状态由调用方按业务决定。

#### Pause / Live 恢复契约

调用方可以依赖以下状态序列：

```text
首次进入 Live:   RecordingStarted
Live → Pause:    Paused
Pause → Live:    Running
Live → Off:      Stopping(SessionEnd)
```

`Running` 的字段在恢复通知中的含义：

- `sample_count`：恢复前已写入的遥测帧数；不包含恢复后即将读取的第一帧。
- `bytes_written`：当前录制文件在磁盘上的近似大小，可能不包含尚未 flush 的缓冲数据。
- `elapsed`：从本次 recording 创建 writer 起经过的墙钟时间，包含暂停时长。
- `fps`：`sample_count / elapsed` 得到的有效平均频率，因此长时间暂停后可能暂时偏低。

当前 `Running` 是状态边沿事件，不是周期性 heartbeat。调用方不应依赖持续收到 `Running` 来判断 ACC 仍处于 Live；应将最近一次 `RecordingStarted`/`Running` 归约为 live，将 `Paused` 归约为 paused，直到收到后续状态事件。

状态发送使用有界 channel 的非阻塞写入。调用方应持续消费 `status_rx` 并提供足够容量，避免 channel 满时丢失包括 `Paused`/`Running` 在内的状态边沿事件。

> 注意：模块暂不替调用方定义最终 UI 状态机。建议调用方至少维护 `connected`、`recording`、`paused`、`last_error` 四个状态。

### RecordingOutcome

每完成一个 ACC session，holder 会向 m1 发送一个 `RecordingOutcome`：

```rust
pub struct RecordingOutcome {
    pub track_name: String,         // e.g. "nurburgring"
    pub car_model: String,          // e.g. "BMW M4 GT3"
    pub session_type: String,       // label: "PRACTICE"|"QUALIFY"|"RACE"|"HOTLAP"|...
    pub session_type_raw: i32,      // physics page 原始值 (0-8)
    pub file_path: PathBuf,         // 录制文件路径
    pub file_size_bytes: u64,       // 文件大小
    pub total_samples: u64,         // 总帧数
    pub duration: Duration,         // 录制总时长
    pub recording_date: String,     // 录制日期 "YYYY/MM/DD" 格式 (UTC+8)
    pub recording_time: String,     // 录制时间 "HH:MM:SS" 格式 (UTC+8)
    pub laps: Vec<LapSummary>,      // 圈速汇总
}

pub struct LapSummary {
    pub lap_number: u32,
    pub is_valid: bool,
    pub lap_time: Option<Duration>,
    pub split_times: Vec<Duration>,
}
```

Session Type 映射表（physics page 0-8）：
| 值 | Label |
|----|-------|
| 0 | PRACTICE |
| 1 | QUALIFY |
| 2 | RACE |
| 3 | HOTLAP |
| 4 | TIME_ATTACK |
| 5 | DRIFT |
| 6 | DRAG |
| 7 | HOTSTINT |
| 8 | HOTLAP_SUPERPOLE |
| 其他 | UNKNOWN |

---

## 3. RecordingController

```rust
impl RecordingController {
    /// 启动长期录制服务。返回后 holder 在后台线程运行。
    /// holder 内部自动管理 ACC 共享内存连接，并为每个 session 发送 outcome。
    pub fn start(
        request: RecordingRequest,
        status_tx: StatusSender,                       // m1 → status_rx
        outcome_tx: OutcomeSender,                     // m1 → outcome_rx
        dashboard_tx: Option<Sender<DashboardValuesFrame>>,  // 可选 dashboard channel
        lap_completed: Option<LapCompletedCallback>,          // 可选圈完成回调
    ) -> TelemetryResult<Self>;

    /// 手动停止。阻塞等待 holder/dashboard 线程退出。
    pub fn stop(&mut self);

    /// 运行时动态添加 dashboard 订阅（非阻塞，发送命令到 dashboard 线程）
    pub fn add_dashboard_item(&self, item: DashboardItemSubscription);

    /// 运行时动态移除 dashboard 订阅
    pub fn remove_dashboard_item(&self, key: &ItemKey);

    /// 原子替换全部 dashboard 订阅
    /// 先同步校验 items，失败时返回 DashboardSubscriptionError 不会发送命令。
    /// 校验成功后发送 dashboard replace command 并更新内部订阅列表。
    pub fn replace_dashboard_items(
        &self,
        items: &[DashboardItemSubscription],
    ) -> Result<(), DashboardSubscriptionError>;

    /// 查询当前已订阅的 dashboard 项信息
    pub fn list_dashboard_items(&self) -> Vec<DashboardItemInfo>;

    /// 查询当前 session 的元数据（track_name、car_model 等）。
    ///
    /// 在收到 RecordingStarted（录制）或 ReplayStarted（回放）之后，
    /// 调用方可主动调用此方法获取 session 信息。session 未开始时返回 None。
    ///
    /// 注意：session_type 在 session 开始时通常为 None，
    /// 该值在录制结束时通过 RecordingOutcome 提供，或从首帧数据中解析。
    pub fn session_metadata(&self) -> Option<SessionMetadata>;
}
```

**生命周期**：面向软件录制生命周期。通常软件启动时调用一次 `start()`，退出或用户停止录制时调用 `stop()`。同一个 controller 可以连续录制多个 session。  
**Drop 行为**：未手动 `stop()` 时，Drop 自动发送停止信号并 join holder/dashboard 线程。  
**并发安全**：重复调用 `stop()` 安全（内部原子标志）。

### Session Metadata 查询

`session_metadata()` 提供一种**不修改 API 协议**的方式，让 m1 在 session 开始时获取 track_name、car_model 等元数据。调用方在收到对应状态后主动查询（拉取模式）。

**录制示例**：

```rust
use module_live_telemetry::recording::*;
use std::time::Duration;
use std::path::PathBuf;

let req = RecordingRequest {
    poll_hz: 60.0,
    output_dir: PathBuf::from("./recordings"),
    status_interval: Duration::from_secs(1),
    dashboard_items: vec![],
    dashboard_realtime_items: vec![],
};
let (status_tx, status_rx) = status_channel(16);
let (outcome_tx, outcome_rx) = outcome_channel();

let mut ctrl = RecordingController::start(req, status_tx, outcome_tx, None, None)?;

// m1 在主线程或任意线程监听状态，收到 RecordingStarted 后查询 metadata
loop {
    match status_rx.recv() {
        Ok(RecordingStatus::RecordingStarted) => {
            if let Some(meta) = ctrl.session_metadata() {
                println!("Session started: {} @ {}", meta.car_model, meta.track_name);
                // meta.poll_hz, meta.sector_count, meta.max_rpm, ...
            }
        }
        Ok(RecordingStatus::Stopping { .. }) => break,
        Ok(_) => {}
        Err(_) => break,
    }
}
```

**回放示例**：

```rust
let req = ReplayRequest {
    file_path: PathBuf::from("./recordings/bmw_nurburgring_1234567890.acctlm2"),
    speed_multiplier: 1.0,
    status_interval: Duration::from_secs(1),
    dashboard_items: vec![],
    dashboard_realtime_items: vec![],
};
let (status_tx, status_rx) = status_channel(16);

let mut ctrl = RecordingController::start_replay(req, status_tx, None, None)?;

// 收到 ReplayStarted 后查询 metadata
loop {
    match status_rx.recv() {
        Ok(RecordingStatus::ReplayStarted) => {
            if let Some(meta) = ctrl.session_metadata() {
                println!("Replaying: {} @ {}", meta.car_model, meta.track_name);
            }
        }
        Ok(RecordingStatus::Stopping { .. }) => break,
        Ok(_) => {}
        Err(_) => break,
    }
}
```

**可用时机**：

| 场景 | 何时可查询 | 备注 |
|------|-----------|------|
| 录制 (`start`) | 收到 `RecordingStarted` 之后 | 每次新 session 开始时 metadata 会更新 |
| 回放 (`start_replay`) | 收到 `ReplayStarted` 之后 | metadata 来自 `.acctlm2` 文件头 |

**返回字段**（`SessionMetadata` 结构体）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `track_name` | `String` | 赛道名称，如 `"nurburgring"` |
| `car_model` | `String` | 车型，如 `"bmw_m4_gt3"` |
| `poll_hz` | `f64` | 录制时的轮询频率 |
| `sector_count` | `i32` | 赛道扇区数 |
| `max_rpm` | `i32` | 车辆最大转速 |
| `max_power` | `f32` | 车辆最大功率 |
| `max_fuel` | `f32` | 油箱容量 (L) |
| `session_type` | `Option<i32>` | session 类型 (0-8)，录制开始时通常为 `None` |
| `created_unix_ns` | `u64` | 录制创建时间 (Unix 纳秒) |
| `sm_version` / `ac_version` | `String` | ACC 版本信息 |
| `num_cars` | `i32` | 场上车辆数 |

> **注意**：`session_type` 在 session **开始时**通常为 `None`。该值在首帧遥测数据中才能获取。录制结束后可通过 `RecordingOutcome.session_type` 获取；回放时也可通过解析帧数据获取。

**竞态安全**：metadata 在 `send_status(RecordingStarted / ReplayStarted)` 之前已完成内部写入，调用方在收到对应状态后查询不会出现竞态。

### DashboardItemInfo

```rust
pub struct DashboardItemInfo {
    pub name: String,          // 如 "raw:controls.speed_kmh"
    pub kind: DashboardItemKind,  // RawItem / CalculatedItem / SystemItem
    pub description: String,   // 人类可读描述
    pub unit: Option<String>,  // 单位，如 "km/h"
}
```

说明：
- 初始值来自 `RecordingRequest.dashboard_items`。
- 调用 `replace_dashboard_items` 成功后会更新为新的订阅列表。
- 调用 `add_dashboard_item` / `remove_dashboard_item` 时也会同步维护该列表。
- raw item 的 `description` / `unit` 会尽量从 raw catalog 补齐。
- calculated item 当前返回通用描述，unit 可能为空。

---

## 4. Channel 规格

| Channel | 方向 | 容量 | 说明 |
|---------|------|------|------|
| `StatusSender→StatusReceiver` | m2→m1 | 16（推荐） | 实时状态推送，非阻塞 try_send |
| `OutcomeSender→OutcomeReceiver` | m2→m1 | 16 | 每个 session 完成时发送一个 outcome；若 m1 不消费且 channel 满，m2 会丢弃该 outcome 并发送 Error 状态 |
| `Sender<DashboardValuesFrame>→Receiver` | m2→m1 | 64（推荐） | Dashboard 数据，每帧按订阅频率产出稀疏结果 |

所有 channel 使用 `crossbeam_channel`，非阻塞发送（m2 不因 m1 消费慢而阻塞录制）。

### DashboardValuesFrame

```rust
pub struct DashboardValuesFrame {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub values: HashMap<String, f64>,   // 稀疏结果，key 为订阅 item name
}
```

- `sample_tick` 和 `timestamp_ns` 来自当前 telemetry frame。
- `values` 是本轮 dashboard 计算出的稀疏结果，key 为订阅 item name。
- 调用方可以使用 `timestamp_ns` 或本地接收时间判断 dashboard 数据是否过期。

---

## 5. Dashboard 订阅

### 基本用法

```rust
use module_live_telemetry::compute::context::ReferenceSource;
use module_live_telemetry::recording::engine::LapCompletedEvent;

let dash_items = vec![
    DashboardItemSubscription::new("raw:controls.speed_kmh", DashboardItemKind::RawItem, Duration::from_millis(50)),
    DashboardItemSubscription::new("raw:controls.rpms",      DashboardItemKind::RawItem, Duration::from_millis(100)),
    // DeltaTimeToLifeBestLap 需要指定参考圈
    DashboardItemSubscription::with_reference(
        "calc:delta_time_to_life_best_lap",
        DashboardItemKind::CalculatedItem,
        Duration::from_millis(100),
        ReferenceSource {
            file_path: PathBuf::from("best_lap.acctlm2"),
            lap_number: 2,
        },
    ),
    // DeltaTimeToSessionBestLap — 运行时动态注入参考圈
    DashboardItemSubscription::new(
        "calc:delta_time_to_session_best_lap",
        DashboardItemKind::CalculatedItem,
        Duration::from_millis(100),
    ),
];

let req = RecordingRequest {
    dashboard_items: dash_items,
    ..req
};

let (dash_tx, dash_rx) = crossbeam_channel::bounded::<DashboardValuesFrame>(64);

let mut best_time = i32::MAX;
let ref_source = ReferenceSource {
    file_path: PathBuf::from("best_lap.acctlm2"),
    lap_number: 1,
};

let on_lap = Box::new(move |event: LapCompletedEvent| {
    if event.is_valid && !event.is_out_lap && event.lap_time_ms < best_time {
        best_time = event.lap_time_ms;
        // 用更快的圈替换 session 最佳圈参考
        // registry.replace_reference(session_best_source, event.lap_frames);
        println!("新Session最佳圈! {}ms", event.lap_time_ms);
    }
});

let mut ctrl = RecordingController::start(
    req,
    status_tx, outcome_tx,
    Some(dash_tx),
    Some(on_lap),
)?;

// m1 侧读取 dashboard 数据
std::thread::spawn(move || {
    while let Ok(frame) = dash_rx.recv() {
        println!("dashboard tick={} ts={} values={:?}",
            frame.sample_tick, frame.timestamp_ns, frame.values);
    }
});
```

### 动态修改 Dashboard

录制过程中可随时增删订阅项：

```rust
use module_live_telemetry::item_key::ItemKey;

// 中途加一项
ctrl.add_dashboard_item(DashboardItemSubscription::new(
    "raw:controls.brake",
    DashboardItemKind::RawItem,
    Duration::from_millis(50),
));

// 中途删一项
ctrl.remove_dashboard_item(&ItemKey::parse("raw:controls.speed_kmh").unwrap());

// 原子替换全部订阅（A,B,C → B,C,D），带回错误反馈
match ctrl.replace_dashboard_items(&[
    DashboardItemSubscription::new("raw:controls.brake",   DashboardItemKind::RawItem, Duration::from_millis(50)),
    DashboardItemSubscription::new("raw:controls.clutch",  DashboardItemKind::RawItem, Duration::from_millis(50)),
    DashboardItemSubscription::new("raw:controls.rpms",    DashboardItemKind::RawItem, Duration::from_millis(100)),
]) {
    Ok(()) => println!("订阅已更新"),
    Err(e) => eprintln!("订阅替换失败: {} — {}", e.item_name, e.message),
}
```

内部通过 channel 发送命令到 dashboard 线程，非阻塞。

### Dashboard 订阅校验

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

校验内容：
- `item_name` 必须能解析为 `raw:*`、`calc:*` 或 `system:*`。
- `item_kind` 必须和 `item_name` 前缀一致。
- `interval` 必须大于 0。
- raw item 必须是有效 telemetry raw field。
- calculated item 必须存在于校验使用的 registry。
- system item 当前不支持。

> **约定**：正常 app 调用 `RecordingController::replace_dashboard_items` 时，以同步 `Result` 作为订阅错误反馈来源。`RecordingStatus` 继续表示 recording lifecycle / shared memory / recording error 等状态。dashboard subscription validation error 不再额外进入 status event 流，避免同一个错误走两套反馈路径。

### 启动时动态注册 Calculated Item

```rust
pub struct DashboardRealtimeItemRegistration {
    pub name: String,
}

impl DashboardRealtimeItemRegistration {
    pub fn new<F>(name: impl Into<String>, factory: F) -> Self
    where
        F: Fn() -> Box<dyn RealtimeComputeItem> + Send + Sync + 'static;
}
```

约定：
- builtin calculated item 不需要调用方注册。
- 启动时动态 calculated item 只需要放入 `dashboard_realtime_items`，不要再在其它位置重复注册。
- `RecordingController` 内部会把 builtin 和启动时动态 item 注册到 dashboard service 实际使用的 `ComputeRegistry`。
- `replace_dashboard_items` 的校验会识别同一批 calculated item。

### 可用 Raw Items

从 `TelemetryFrame` 直接映射的字段：

| Name | 来源 | 单位 |
|------|------|------|
| `speed_kmh` | `controls.speed_kmh` | km/h |
| `gas` | `controls.gas` | 0.0-1.0 |
| `brake` | `controls.brake` | 0.0-1.0 |
| `clutch` | `controls.clutch` | 0.0-1.0 |
| `steer_angle` | `controls.steer_angle` | rad |
| `gear` | `controls.gear` | -1/0/1-6 |
| `rpms` | `controls.rpms` | rpm |
| `fuel` | `controls.fuel` | L |

> 完整 raw item 列表见 [raw-item.md](raw-item.md)。

### 可用 Calculated Items

通过 `list_available_items()` 或 `all_builtin_calculated_items()` 动态发现。

```rust
use module_live_telemetry::compute::items::all_builtin_calculated_items;

// 查询所有内置 calculated item
let items = all_builtin_calculated_items();
for item in &items {
    println!("  {} — {} [{:?}] 需要参考圈: {}",
        item.key, item.description, item.unit, item.requires_reference);
}
// 输出包含:
// calc:delta_time_to_life_best_lap — 当前圈与历史最佳圈时间差 [ms] 需要参考圈: true
// calc:delta_time_to_session_best_lap — 当前圈与本Session最佳圈时间差 [ms] 需要参考圈: true
// calc:prev_sector_time / calc:prev_sector_number / calc:sector_best_1..3
```

| 内置项 | 描述 | 单位 | 需要参考圈 |
|--------|------|------|------------|
| `calc:delta_time_to_life_best_lap` | 当前圈与历史最佳圈时间差 | ms | 是（外部文件） |
| `calc:delta_time_to_session_best_lap` | 当前圈与本Session最佳圈时间差 | ms | 是（运行时动态注入） |
| `calc:prev_sector_time` | 上一个 Sector 耗时 | ms | 否 |
| `calc:prev_sector_number` | 上一个 Sector 编号 | — | 否 |
| `calc:sector_best_1` | Sector 1 最佳耗时 | ms | 否 |
| `calc:sector_best_2` | Sector 2 最佳耗时 | ms | 否 |
| `calc:sector_best_3` | Sector 3 最佳耗时 | ms | 否 |

> 扇区计算项详细逻辑见 [reference/sector-calculated-items.md](../reference/sector-calculated-items.md)。

---

## 6. 错误处理

```rust
pub enum TelemetryError {
    Io(std::io::Error),
    InvalidFormat(String),
    UnsupportedVersion(u16),
    InvalidArgument(String),  // 参数校验失败、目录不存在等
}
```

常见错误场景：
| 场景 | 错误 |
|------|------|
| poll_hz 越界 | `InvalidArgument("poll_hz must be between 30.0 and 120.0")` |
| output_dir 不存在 | `InvalidArgument("output directory does not exist")` |
| ACC 未运行 / shared memory 尚不可用 | `RecordingStatus::WaitingForSharedMemory { message }`，holder 继续等待 |
| 非 Windows 环境 | holder 后台发送 `WaitingForSharedMemory`；`start()` 本身仍可成功返回 controller |
| 录制中共享内存断连 | 当前 session 尝试 finish，发送 `Stopping { reason: ShmemLost }`，随后 holder 回到等待/重连循环 |
| outcome channel 已满 | 丢弃该 session outcome，并发送 `RecordingStatus::Error { kind: Unknown }` |

---

## 7. 文件命名规则

m1 传入目录，m2 自动命名：`{car}_{track}_{timestamp}.acctlm2`

- 文件名中非法字符（`<>:"/\|?*`）替换为 `_`
- 空格替换为 `_`
- 时间戳为 Unix 秒
- 文件已存在时自动追加 `-N` 后缀，例如 `car_track_123.acctlm2` → `car_track_123-1.acctlm2`，不覆盖

---

## 8. 并发模型

```
m1 线程                          m2 holder 线程                    dashboard 线程
   |                                  |                                  |
   |-- start() ────────────────────→ spawn holder                        |
   |←─ return controller              |                                  |
   |←─ status_tx ─── Started ─────────|                                  |
   |←─ status_tx ─── Waiting* ────────|  ACC 未启动/未连上               |
   |                                  |-- open shared memory loop         |
   |←─ status_tx ─── Connected ───────|                                  |
   |                                  |-- run one-session engine           |
   |←─ status_tx ─── RecordingStarted |                                  |
   |←─ dash_tx ─── DashboardValuesFrame* ──── frame channel ───────────→ |
   |←─ status_tx ─── Paused/Stopping  |                                  |
   |←─ outcome_tx ─── Outcome(session)|                                  |
   |                                  |-- loop back, wait for next session |
   |-- stop() ─────────────────────→  | finish current writer if needed   |
   |   join()                         | drop frame/cmd senders            |
   |                                  X────────────────────────────────→ X
```

- m1 调用全部同步，无需 async/tokio
- `RecordingController::start()` 不等待 ACC shared memory；只校验参数并启动 holder
- m2 使用 `std::thread` + `crossbeam_channel`
- status/dashboard/outcome 发送使用非阻塞 `try_send`，m1 消费慢不阻塞录制
- dashboard 线程跟随 controller 生命周期创建一次；不同 session 共用同一套 dashboard 订阅

---

## 9. 测试

`RecordingController::start()` 不再同步打开 `AccTelemetrySource`，因此测试可以在无 ACC shared memory 的环境中验证 holder 启动/停止。

```rust
// 测试仅验证参数校验（不依赖 ACC）
let dir = std::env::temp_dir();
let req = RecordingRequest {
    poll_hz: 60.0,
    output_dir: dir,
    status_interval: Duration::from_secs(1),
    dashboard_items: vec![],
    dashboard_realtime_items: vec![],
};
let (status_tx, _) = status_channel(8);
let (outcome_tx, _) = outcome_channel();

let mut controller = RecordingController::start(req, status_tx, outcome_tx, None, None)?;
controller.stop();
```

---

## 10. 模块依赖

```
m1 依赖:
  module_live_telemetry = { path = "../module_live_telemetry" }
  crossbeam-channel = "0.5"

m1 使用的 public API 路径:
  module_live_telemetry::recording::{
      RecordingRequest, RecordingStatus, RecordingOutcome,
      RecordingController, RecordingErrorKind, StopReason,
      status_channel, outcome_channel,
      DashboardItemSubscription, DashboardItemKind,
      DashboardItemInfo, DashboardValuesFrame,
      DashboardSubscriptionError, validate_dashboard_subscriptions,
      engine::{LapCompletedEvent, LapCompletedCallback},
  }
  module_live_telemetry::types::SessionMetadata    // session_metadata() 返回类型
  module_live_telemetry::item_key::ItemKey
  module_live_telemetry::compute::context::ReferenceSource
  module_live_telemetry::TelemetryResult, TelemetryError
```

---

## 11. Replay API

回放任一已录制的 `.acctlm2` 文件，以模拟实时遥测数据流的方式驱动 dashboard 计算。回放在独立 holder 线程中运行，不依赖 ACC shared memory，也不产生录制文件。

### ReplayRequest

```rust
pub struct ReplayRequest {
    /// 录制文件路径，必须存在
    pub file_path: PathBuf,
    /// 回放倍速，1.0 = 原始速度，2.0 = 二倍速
    pub speed_multiplier: f64,
    /// 状态回调间隔
    pub status_interval: Duration,
    /// dashboard 订阅项
    pub dashboard_items: Vec<DashboardItemSubscription>,
    /// 启动时动态注册的 realtime calculated dashboard item
    pub dashboard_realtime_items: Vec<DashboardRealtimeItemRegistration>,
}
```

`validate()` 校验规则：

| 校验项 | 约束 |
|--------|------|
| `file_path` | 必须存在且为文件 |
| `speed_multiplier` | 必须 > 0.0 且有限（NaN/Inf 拒绝） |
| `status_interval` | 必须 > 0 |

### 回放状态流

与录制不同，回放不会发送 `Connected`、`WaitingForSharedMemory`、`Paused` 或 `RecordingStarted`。状态序列如下：

```text
Started → ReplayStarted → Running* → Stopping(FramesExhausted|Manual)
```

- `ReplayStarted`：回放数据流已启动，开始从文件中读取帧。
- `Running`：周期性进度更新。`bytes_written` 始终为 0（回放不写入文件）。
- `Stopping(FramesExhausted)`：文件所有帧已回放完毕，自然结束。
- `Stopping(Manual)`：调用方手动 `stop()` 结束。

```rust
pub enum RecordingStatus {
    // ... 现有变体 ...
    ReplayStarted,            // 回放数据流已启动
    // ...
    Stopping { reason: StopReason },
}

pub enum StopReason {
    Manual,           // 手动停止
    SessionEnd,       // ACC session 结束
    ShmemLost,        // 共享内存连接丢失
    FramesExhausted,  // 所有帧已回放完毕
}
```

**重要区别**：
- 回放**不发送** `RecordingOutcome`（没有文件被录制）。
- 回放**不发送** `Connected`、`WaitingForSharedMemory`、`Paused`、`RecordingStarted`。
- `Running` 的 `bytes_written` 字段始终为 0。

### RecordingController 回放方法

```rust
impl RecordingController {
    /// 启动回放服务。返回后 holder 在后台线程运行。
    pub fn start_replay(
        request: ReplayRequest,
        status_tx: StatusSender,                       // m1 → status_rx
        dashboard_tx: Option<Sender<DashboardValuesFrame>>,  // 可选 dashboard channel
        lap_completed: Option<LapCompletedCallback>,          // 可选圈完成回调
    ) -> TelemetryResult<Self>;

    /// 启动回放，使用 overwrite-on-full dashboard channel。
    /// 推荐用于 live HUD 消费者：消费者落后时只保留最新的稀疏帧。
    pub fn start_replay_with_latest_dashboard(
        request: ReplayRequest,
        status_tx: StatusSender,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<(Self, LatestValueReceiver)>;
}
```

**生命周期**：面向单次回放生命周期。调用一次回放一个文件，完成后 `stop()` 或等待自然结束。未 `stop()` 时 Drop 自动发送停止信号并 join 线程。重复调用 `stop()` 安全。

### 使用示例

```rust
use module_live_telemetry::recording::*;
use std::time::Duration;
use std::path::PathBuf;

fn main() -> TelemetryResult<()> {
    // 1. 构造回放请求
    let req = ReplayRequest {
        file_path: PathBuf::from("./recordings/bmw_nurburgring_1234567890.acctlm2"),
        speed_multiplier: 2.0,                        // 2倍速回放
        status_interval: Duration::from_secs(1),
        dashboard_items: vec![
            DashboardItemSubscription::new(
                "raw:controls.speed_kmh",
                DashboardItemKind::RawItem,
                Duration::from_millis(50),
            ),
            DashboardItemSubscription::new(
                "raw:controls.rpms",
                DashboardItemKind::RawItem,
                Duration::from_millis(100),
            ),
        ],
        dashboard_realtime_items: vec![],
    };

    // 2. 创建 channel
    let (status_tx, status_rx) = status_channel(16);

    // 3. 启动回放（使用 latest dashboard，适合 HUD）
    let (mut ctrl, dash_rx) = RecordingController::start_replay_with_latest_dashboard(
        req,
        status_tx,
        None,  // 无圈完成回调
    )?;

    // 4. 读取 dashboard 数据（最新值覆盖模式）
    let dash_handle = std::thread::spawn(move || {
        loop {
            match dash_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => {
                    if let Some(speed) = frame.values.get("raw:controls.speed_kmh") {
                        println!("速度: {:.1} km/h", speed);
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    // 5. 主线程读取状态，在 ReplayStarted 时查询 session metadata
    while let Ok(status) = status_rx.recv() {
        match status {
            RecordingStatus::Started { .. } => println!("回放服务已启动"),
            RecordingStatus::ReplayStarted => {
                if let Some(meta) = ctrl.session_metadata() {
                    println!("开始回放: {} @ {}，{} 扇区",
                        meta.car_model, meta.track_name, meta.sector_count);
                }
            }
            RecordingStatus::Running { sample_count, elapsed, fps, .. } => {
                println!("回放中: {} 帧, 已运行 {:?}, {:.1} Hz", sample_count, elapsed, fps);
            }
            RecordingStatus::Stopping { reason } => {
                println!("正在停止: {}", reason);
                break;
            }
            RecordingStatus::Error { message, .. } => {
                eprintln!("错误: {}", message);
            }
            _ => {}
        }
    }

    dash_handle.join().unwrap();

    // 6. 等待回放自然结束或手动停止
    // ctrl.stop();

    Ok(())
}
```
