# RecordingController API 参考

m2 暴露给 m1 的录制控制接口。m1 通过同步调用 + 独立 channel 模式启动/停止 ACC 遥测录制。

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
        enable_dashboard: false,
        dashboard_items: vec![],
    };

    // 2. 创建 channel（m1 侧接收端）
    let (status_tx, status_rx) = status_channel(16);  // 状态
    let (outcome_tx, outcome_rx) = outcome_channel(); // 结束回调

    // 3. 启动录制
    let source = AccTelemetrySource::open()?;  // 生产环境用 ACC 共享内存
    let mut ctrl = RecordingController::start(
        req, source,
        status_tx, outcome_tx,
        None,  // 无 dashboard
        None,  // 无圈完成回调
    )?;

    // 4. 读取状态
    std::thread::spawn(move || {
        while let Ok(status) = status_rx.recv() {
            match status {
                RecordingStatus::Connected => println!("ACC 已连接"),
                RecordingStatus::RecordingStarted => println!("开始录制"),
                RecordingStatus::Running { sample_count, bytes_written, elapsed, fps } => {
                    println!("录制中... {sample_count} 帧, {fps:.1} Hz");
                }
                RecordingStatus::Stopping { reason } => {
                    println!("正在停止: {reason}");
                    break;
                }
                RecordingStatus::Error { message, .. } => {
                    eprintln!("错误: {message}");
                    break;
                }
                _ => {}
            }
        }
    });

    // 5. 获取结果（阻塞等待录制结束）
    let outcome = outcome_rx.recv()?;
    println!("录制完成: {}", outcome.file_path.display());
    println!("  track: {}", outcome.track_name);
    println!("  car:   {}", outcome.car_model);
    println!("  type:  {}", outcome.session_type);
    println!("  date:  {}", outcome.recording_date);
    println!("  time:  {}", outcome.recording_time);
    println!("  laps:  {}", outcome.laps.len());
    println!("  size:  {} bytes", outcome.file_size_bytes);

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
    /// 是否启用 dashboard 数据回传
    pub enable_dashboard: bool,
    /// dashboard 订阅项
    pub dashboard_items: Vec<DashboardItemSubscription>,
}
```

`validate()` 校验规则：
| 校验项 | 约束 |
|--------|------|
| `poll_hz` | `30.0 <= poll_hz <= 120.0`，必须有限（NaN/Inf 拒绝） |
| `output_dir` | 必须存在且为目录 |
| `status_interval` | 必须 > 0 |
| 文件名碰撞 | 自动命名文件已存在 → 返回 `Err(InvalidArgument)`，不覆盖 |

### RecordingStatus

m2 → m1 的状态通知流：

```
Connected → RecordingStarted → Running* → Stopping → (channel close)
                    ↓                        ↑
                  Paused ←→ Running          |
                    ↓                        |
                  Error ─────────────────────┘
```

```rust
pub enum RecordingStatus {
    Started { thread_id: u64 },  // 后台线程已启动（但可能尚未连接 ACC）
    Connected,                   // ACC 共享内存已连接
    RecordingStarted,            // writer 已创建，帧数据开始写入
    Running {                    // 定期进度更新
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

### RecordingOutcome

录制结束时 m2 → m1 的一次性回调：

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
    /// 启动录制。返回后录制在后台线程运行。
    pub fn start<S: TelemetrySource + 'static>(
        request: RecordingRequest,
        source: S,                      // AccTelemetrySource（生产）或 ScriptedTelemetrySource（测试）
        status_tx: StatusSender,        // m1 → status_rx
        outcome_tx: OutcomeSender,      // m1 → outcome_rx
        dashboard_tx: Option<Sender<HashMap<String, f64>>>,  // 可选 dashboard channel
        lap_completed: Option<LapCompletedCallback>,          // 可选圈完成回调
    ) -> TelemetryResult<Self>;

    /// 手动停止。阻塞等待录制线程退出。
    pub fn stop(&mut self);

    /// 运行时动态添加 dashboard 订阅
    pub fn add_dashboard_item(&self, item: DashboardItemSubscription);

    /// 运行时动态移除 dashboard 订阅
    pub fn remove_dashboard_item(&self, key: &ItemKey);

    /// 原子替换全部 dashboard 订阅
    pub fn replace_dashboard_items(&self, items: &[DashboardItemSubscription]);
}
```

**生命周期**：单次使用。每次调用 `start()` 创建新实例。  
**Drop 行为**：未手动 `stop()` 时，Drop 自动发送停止信号并 join 线程。  
**并发安全**：重复调用 `stop()` 安全（内部原子标志）。

---

## 4. Channel 规格

| Channel | 方向 | 容量 | 说明 |
|---------|------|------|------|
| `StatusSender→StatusReceiver` | m2→m1 | 16（推荐） | 实时状态推送，非阻塞 try_send |
| `OutcomeSender→OutcomeReceiver` | m2→m1 | 1 | 录制结束一次性回调 |
| `Sender<HashMap<String,f64>>→Receiver` | m2→m1 | 64（推荐） | Dashboard 数据，按频率聚合 |

所有 channel 使用 `crossbeam_channel`，非阻塞发送（m2 不因 m1 消费慢而阻塞录制）。

---

## 5. Dashboard 订阅

```rust
// 设置 dashboard 订阅 + 圈完成回调
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
            file_path: PathBuf::from("best_lap.acctlm"),
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
    enable_dashboard: true,
    dashboard_items: dash_items,
    ..req
};

let (dash_tx, dash_rx) = crossbeam_channel::bounded::<HashMap<String, f64>>(64);

    let mut best_time = i32::MAX;
    let ref_source = ReferenceSource {
        file_path: PathBuf::from("best_lap.acctlm"),
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
        req, source,
        status_tx, outcome_tx,
        Some(dash_tx),
        Some(on_lap),
    )?;

// m1 侧读取 dashboard 数据
std::thread::spawn(move || {
    while let Ok(data) = dash_rx.recv() {
        for (name, value) in &data {
            println!("  {name} = {value:.2}");
        }
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

// 原子替换全部订阅（A,B,C → B,C,D）
ctrl.replace_dashboard_items(&[
    DashboardItemSubscription::new("raw:controls.brake",   DashboardItemKind::RawItem, Duration::from_millis(50)),
    DashboardItemSubscription::new("raw:controls.clutch",  DashboardItemKind::RawItem, Duration::from_millis(50)),
    DashboardItemSubscription::new("raw:controls.rpms",    DashboardItemKind::RawItem, Duration::from_millis(100)),
]);
```

内部通过 channel 发送命令到 dashboard 线程，非阻塞。

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
// 输出: calc:delta_time_to_life_best_lap — 当前圈与参考圈时间差 [ms] 需要参考圈: true
```

| 内置项 | 描述 | 单位 | 需要参考圈 |
|--------|------|------|------------|
| `calc:delta_time_to_life_best_lap` | 当前圈与历史最佳圈时间差 | ms | 是（外部文件） |
| `calc:delta_time_to_session_best_lap` | 当前圈与本Session最佳圈时间差 | ms | 是（运行时动态注入） |

自定义 calculated item 先注册到 `ComputeRegistry` 再通过 `list_available_items()` 发现：

```rust
// 注册后可见
registry.register_calc_realtime(Box::new(MyItem)).unwrap();

let service = DashboardService::new(registry, Box::new(dummy_sink));
let items = service.list_available_items();  // raw + calc 合并
```

---

## 6. 错误处理

```rust
pub enum TelemetryError {
    Io(std::io::Error),
    InvalidFormat(String),
    UnsupportedVersion(u16),
    InvalidArgument(String),  // 校验失败、目录不存在、文件碰撞
}
```

常见错误场景：
| 场景 | 错误 |
|------|------|
| poll_hz 越界 | `InvalidArgument("poll_hz must be between 30.0 and 120.0")` |
| output_dir 不存在 | `InvalidArgument("output directory does not exist")` |
| 文件已存在 | `InvalidArgument("output file already exists")` |
| ACC 未运行 (非 Windows) | `InvalidArgument("ACC shared memory is only available on Windows")` |
| 录制中共享内存断连 | `RecordingStatus::Error { kind: ShmemDisconnected }` |

---

## 7. 文件命名规则

m1 传入目录，m2 自动命名：`{car}_{track}_{timestamp}.acctlm`

- 文件名中非法字符（`<>:"/\|?*`）替换为 `_`
- 空格替换为 `_`
- 时间戳为 Unix 秒
- 文件已存在时 `start()` 返回错误，不覆盖

---

## 8. 并发模型

```
m1 线程                          m2 录制线程
   |                                |
   |-- start() ─────────────────→ spawn
   |                                |
   |←─ status_tx ──── Connected ───|
   |←─ status_tx ──── RecordingStarted
   |←─ status_tx ──── Running* ────|  (定期)
   |←─ dash_tx ────── HashMap* ───|  (按频率)
   |                                |
   |-- stop() ────────────────────→|
   |   join()                       | finish writer
   |←─ status_tx ──── Stopping ────|
   |←─ outcome_tx ─── Outcome ─────|
   |                                X
```

- m1 调用全部同步，无需 async/tokio
- m2 使用 `std::thread` + `crossbeam_channel`
- status/dashboard channel 使用 `try_send`，m1 消费慢不阻塞录制

---

## 9. 测试

使用 `ScriptedTelemetrySource` 进行无 ACC 集成测试：

```rust
use module_live_telemetry::recording::source::{ScriptedTelemetrySource, ScriptedStep};
use module_live_telemetry::shmem::AccGameStatus;

let steps = vec![
    ScriptedStep::new().with_status(AccGameStatus::Live).with_frame(make_frame(0, 100.0)),
    ScriptedStep::new().with_status(AccGameStatus::Live).with_frame(make_frame(1, 150.0)),
    ScriptedStep::new().with_status(AccGameStatus::Off),
];
let fake_source = ScriptedTelemetrySource::new(steps);

let mut ctrl = RecordingController::start(req, fake_source, status_tx, outcome_tx, None, None)?;
let outcome = outcome_rx.recv_timeout(Duration::from_secs(5))?;
assert_eq!(outcome.total_samples, 2);
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
      engine::{LapCompletedEvent, LapCompletedCallback},
      AccTelemetrySource, SessionTypeLabel,
  }
  module_live_telemetry::compute::context::ReferenceSource
  module_live_telemetry::TelemetryResult, TelemetryError
```
