# 圈完成回调 API

> **相关文档**: [recording.md](recording.md) · [calculated-item.md](calculated-item.md)

## 概述

在实时录制或 dashboard 过程中，当车辆完成一圈时，系统通过回调通知调用方。回调返回圈的有效性和圈速。

检测方式：监控 ACC 的 `completed_laps` 计数器变化——当计数器递增时，表示一圈完成。

## 数据结构

### LapCompletedEvent

```rust
pub struct LapCompletedEvent {
    /// 圈号（0 = 出站圈，1 = 第一圈，以此类推）
    pub lap_number: u32,
    /// ACC 判定该圈是否有效
    pub is_valid: bool,
    /// 圈速（毫秒）
    pub lap_time_ms: i32,
    /// 是否为出站圈（从维修区驶出的第一圈）
    pub is_out_lap: bool,
    /// 刚完成圈的完整帧数据。
    /// 可用于更新参考圈：`registry.replace_reference(source, event.lap_frames)`
    pub lap_frames: Vec<TelemetryFrame>,
}
```

### 回调类型

```rust
pub type LapCompletedCallback = Box<dyn Fn(LapCompletedEvent) + Send>;
```

## 使用方式

### 通过 RecordingController

```rust
use module_live_telemetry::recording::{
    RecordingController, RecordingRequest,
    engine::LapCompletedEvent,
};
use std::time::Duration;

let request = RecordingRequest {
    poll_hz: 60.0,
    output_dir: "/path/to/output".into(),
    status_interval: Duration::from_secs(1),
    dashboard_items: vec![],
    dashboard_realtime_items: vec![],
};

// 定义回调
let on_lap_completed: Box<dyn Fn(LapCompletedEvent) + Send> = Box::new(|event| {
    println!(
        "圈 {} 完成 — 有效: {}, 时间: {}ms, 出站圈: {}",
        event.lap_number,
        event.is_valid,
        event.lap_time_ms,
        event.is_out_lap,
    );
});

let controller = RecordingController::start(
    request,
    status_tx,
    outcome_tx,
    None,                     // 无 dashboard 数据通道
    Some(on_lap_completed),   // ← 圈完成回调
)?;
```

### 通过 run_recording_loop（底层 API）

```rust
use module_live_telemetry::recording::engine::{
    run_recording_loop, RecordingEngineConfig, LapCompletedEvent,
};

let config = RecordingEngineConfig {
    poll_hz: 60.0,
    poll_interval: Duration::from_secs_f64(1.0 / 60.0),
    chunk_rows: 256,
    status_interval: Duration::from_secs(1),
    flush_interval: Some(Duration::from_secs(2)),
};

// 不需要回调时传 None
run_recording_loop(config, &mut source, output_path, None, stop_rx, None)?;

// 需要回调时
run_recording_loop(
    config,
    &mut source,
    output_path,
    None,
    stop_rx,
    Some(Box::new(|event| {
        // event.lap_number, event.is_valid, event.lap_time_ms
    })),
)?;
```

## 检测逻辑

系统通过 `TelemetryFrame.session.completed_laps` 字段检测圈完成：

1. 每帧记录 `completed_laps` 值
2. 当该值递增时，触发回调
3. `lap_time_ms` 取自 `TelemetryFrame.timing.i_last_time`（ACC 提供的是已完成圈的时间，单位 ms）
4. `is_out_lap` 在 `lap_number == 0` 时为 `true`

## 注意事项

- 回调在录制引擎线程中**同步执行**，请确保回调不阻塞
- 如需将事件发送到其他线程，在回调内通过 channel 转发
- 出站圈（lap 0）的时间通常不可靠，由调用方决定是否使用
- `lap_frames` 在圈完成时自动清零，下一圈重新累积

## 动态更新参考圈

结合 `ComputeRegistry::replace_reference()`，运行时用更快的圈替换参考圈：

```rust
use module_live_telemetry::{
    compute::{ComputeRegistry, context::ReferenceSource},
    recording::engine::LapCompletedEvent,
};

let mut best_time_ms: i32 = i32::MAX;
let ref_source = ReferenceSource {
    file_path: PathBuf::from("best_lap.acctlm2"),
    lap_number: 1,
};

let on_lap = Box::new(move |event: LapCompletedEvent| {
    // 用刚完成的更快圈替换参考圈
    if event.is_valid && !event.is_out_lap && event.lap_time_ms < best_time_ms {
        best_time_ms = event.lap_time_ms;
        // 通过 dashboard service 的 registry_mut() 更新
        // registry.replace_reference(ref_source.clone(), event.lap_frames);
        println!("new best lap! {} ms", event.lap_time_ms);
    }
});

RecordingController::start(request, status_tx, outcome_tx, None, Some(on_lap))?;
```

## 回调通道示例

将圈完成事件转发到外部 channel：

```rust
use crossbeam_channel::bounded;

let (lap_tx, lap_rx) = bounded::<LapCompletedEvent>(16);

let on_lap = Box::new(move |event| {
    // 非阻塞发送，队列满时丢弃
    let _ = lap_tx.try_send(event);
});

RecordingController::start(request, status_tx, outcome_tx, None, Some(on_lap))?;

// 在另一个线程中接收
std::thread::spawn(move || {
    while let Ok(event) = lap_rx.recv() {
        // 处理圈完成事件
    }
});
```
