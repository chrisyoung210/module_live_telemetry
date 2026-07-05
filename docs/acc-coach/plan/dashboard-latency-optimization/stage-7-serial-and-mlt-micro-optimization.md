# 阶段 7：Serial Dashboard + module_live_telemetry 微优化

日期：2026-06-29
涉及模块：`acc-coach`（serial 路径）+ `module_live_telemetry`（微优化，告知用户）
依赖：阶段6（CompactPatch 基础设施）

---

## 1. 目标

- Serial 路径复用 CompactPatch + LEB128 压缩，使高频遥测在串口上可行
- module_live_telemetry 侧微优化（低优先级，告知用户）

## 2. Serial Dashboard 路径

### 2.1 背景

串口 115200 baud ≈ 14KB/s。当前 serial 路径是 stub（`DashboardTransport` trait 未实现）。

### 2.2 改动方案

**改动文件**：`src/dashboard/remote/protocol.rs`（或新建 serial 模块）

复用阶段6的 CompactPatch，附加 LEB128 压缩：

```rust
// CompactPatch to_bytes() 当前用 fixed 12B per field (4B id + 8B f64)
// Serial 版本可用 LEB128 编码 id（小 id 用 1 byte）+ f32 代替 f64

pub fn to_bytes_compact(&self) -> Vec<u8> {
    let mut output = Vec::with_capacity(16 + self.values.len() * 6);
    output.extend_from_slice(&self.subscription_generation.to_le_bytes());
    output.extend_from_slice(&self.sample_tick.to_le_bytes());
    output.extend_from_slice(&self.timestamp_ns.to_le_bytes());
    output.extend_from_slice(&(self.values.len() as u32).to_le_bytes());
    for (id, value) in &self.values {
        write_leb128_u32(&mut output, *id);       // 1-5 bytes
        output.extend_from_slice(&(*value as f32).to_le_bytes());  // 4 bytes (f32 替代 f64)
    }
    output
}
```

**预估 payload**（15 字段，ID < 128）：
- fixed: 28B header + 15×12B = 208B
- compact: 28B header + 15×5B = 103B
- 串口传输时间：103B / 14KB/s ≈ 7ms（vs JSON 400B ≈ 28ms）

### 2.3 Delta + Keyframe 策略

参考 `docs/acc-coach/dashboard-architecture.md` 8.3 节：

| payload_type | 含义 | 使用场景 |
|---|---|---|
| `0x01` | telemetry_snapshot | 初始连接、keyframe 间隔 |
| `0x02` | telemetry_delta | 高频数据，仅发送变化字段 |
| `0x03` | keyframe | delta 模式下周期锚点 |

Delta 帧：只发送自上次发送以来值变化的字段。设备端维护当前状态，delta 更新状态。

Keyframe 间隔：根据 baud rate 调整，如 115200 baud 下每 2s 发一次完整快照。

### 2.4 验收标准

| 验收项 | 验证方法 |
|---|---|
| compact 编码正确 | 单元测试：encode → decode → 比较 |
| payload size | 对比 fixed vs compact vs JSON |
| delta 编码正确 | 单元测试：连续帧 delta 只含变化字段 |
| serial stub 接口 | `DashboardTransport::send_frame` 返回 NotImplemented → 改为实际编码 |

## 3. module_live_telemetry 微优化（告知用户）

### 3.1 B1：process_frame 避免不必要 clone

**位置**：`src/dashboard/service.rs:615`

```rust
// 当前
if let Err(err) = self.sink.send(dashboard_frame.clone()) {
```

`clone` 是为了后续 `debug_trace.write_frame(self.sent_frame_count, &dashboard_frame, value_count)` 引用 `dashboard_frame`。

**优化**：当 debug trace 禁用时（默认），不需要 clone：

```rust
let trace_enabled = self.debug_trace.is_active();
let frame_to_send = if trace_enabled {
    dashboard_frame.clone()
} else {
    dashboard_frame
};
if let Err(err) = self.sink.send(frame_to_send) {
    // ...
}
if trace_enabled {
    self.debug_trace.write_frame(self.sent_frame_count, &dashboard_frame, value_count);
}
```

**收益**：微优化，每帧少一次 HashMap clone（debug trace 禁用时）。

### 3.2 B3：录制循环读取粒度

**位置**：`src/recording/source.rs:100-109`

当前 `read_all_telemetry_frame` 每次读 physics + graphics 两个共享内存页。高频 HUD 字段（speed/rpm/gear）只需 physics 页。

**优化方向**：分离读取路径，dashboard 线程只读所需页面。但架构改动大（TelemetrySource trait 变化），收益与复杂度不成比例。

**建议**：暂不实施，除非有明确的性能瓶颈证据。

## 4. 模块间开发顺序

| 子阶段 | 模块 | 依赖 |
|---|---|---|
| Serial compact 编码 | acc-coach | 阶段6 CompactPatch |
| B1 clone 优化 | module_live_telemetry | 无（独立） |

两者可并行。Serial 编码是 acc-coach 内部改动；B1 是 module_live_telemetry 内部改动。

## 5. 验收标准

| 验收项 | 验证方法 |
|---|---|
| compact 编码正确 | 单元测试 |
| B1 clone 优化 | module_live_telemetry 单元测试通过，dashboard 输出不变 |
| 无功能回归 | 全链路回归测试 |

## 6. 参照文档

- [stage-6-remote-dashboard-id-protocol.md](./stage-6-remote-dashboard-id-protocol.md) — CompactPatch 基础
- `docs/acc-coach/dashboard-architecture.md` — 8. Serial 预留设计
- README.md 关键代码位置索引 — `DashboardService::process_frame` 条目
