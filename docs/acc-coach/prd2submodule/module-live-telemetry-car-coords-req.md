# module_live_telemetry: 暴露 carCoordinates 需求规格

## 背景

ACC Coach 需要获取赛车在赛道上的世界坐标（X, Z）以支持：
- **Map Widget**：在 Dashboard 上实时显示赛车在赛道轮廓图中的位置
- **Track Map Auto-Collect**：跑圈时自动采集赛车线轨迹

## 需求

### 1. 在 TelemetryFrame 中新增两个字段

```rust
pub struct TelemetryFrame {
    // ... existing fields ...

    /// 玩家赛车的世界坐标 X（米），从 Graphics 页 carCoordinates[0] 提取
    pub car_x: f32,

    /// 玩家赛车的世界坐标 Z（米），从 Graphics 页 carCoordinates[2] 提取
    pub car_z: f32,
}
```

### 2. 数据来源

- **来源**：ACC Graphics 共享内存页（`Local\acpmf_graphics`）中的 `carCoordinates` 字段
- **格式**：`[f32; 180]` = 60 辆车 × 3 轴（X, Y, Z），玩家车 = index 0
- **提取逻辑**（参考 `src/trackmap.rs:84-88` 的现有模式）：
  ```rust
  let base = 0; // player car = index 0
  let car_x = gfx.car_coordinates[base];      // X
  // let car_y = gfx.car_coordinates[base + 1]; // Y (elevation, not needed)
  let car_z = gfx.car_coordinates[base + 2];  // Z
  ```

### 3. 需要修改的文件

| 文件 | 修改内容 |
|------|----------|
| `src/types.rs` | 在 `TelemetryFrame` 结构体（或其他包含 car_coordinates 的结构体）中添加 `car_x: f32`, `car_z: f32` |
| `src/shmem.rs` | 在 ACC Graphics 页读取逻辑中（`TelemetryFrame { ... }` 构造处），从 `gfx.car_coordinates` 提取 car_x/car_z |
| 所有 `TelemetryFrame` 构造函数 | `reader.rs`, `reader_v2.rs`, `writer.rs`, `distributor.rs`, `recording/source.rs` 等 —— 需要补充 `car_x: 0.0, car_z: 0.0` 默认值 |
| 测试文件 | `tests/` 下构造 TelemetryFrame 的测试辅助函数（如 `make_frame()`） |

### 4. 向后兼容

- 新增字段使用默认值 `0.0`（不影响现有序列化/反序列化逻辑）
- 不改变现有 `car_coordinates: Vec<f32>` 字段
- 不修改录制格式 v1/v2 的二进制布局

### 5. 验收标准

- [ ] `cargo test` 全部通过
- [ ] 构造 `car_coordinates = [100.0, 5.0, 200.0, ...]` 的帧 → `car_x == 100.0, car_z == 200.0`
- [ ] 空/零 carCoordinates → `car_x == 0.0, car_z == 0.0`（不崩溃）
- [ ] 所有现有 TelemetryFrame 构造点已更新

### 6. 不需要做的

- ❌ 不需要暴露 Y 轴（elevation）
- ❌ 不需要暴露对手车坐标（index 1-59）
- ❌ 不需要添加新的共享内存读取逻辑（Graphics 页已在读取）
- ❌ 不需要修改 ACC Coach 端代码（由 ACC Coach 团队自行适配）

---

## 接口契约（ACC Coach 侧依赖）

ACC Coach 会在 `src/live/shared_memory.rs` 的 `LiveFrame` 中新增：

```rust
pub car_x: Option<f64>,  // 从 TelemetryFrame.car_x 映射
pub car_z: Option<f64>,  // 从 TelemetryFrame.car_z 映射
```

ACC Coach 期望 `TelemetryFrame` 提供 `car_x: f32` 和 `car_z: f32` 两个公开字段。
