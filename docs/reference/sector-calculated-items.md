# Sector Calculated Item 详细说明

本文档详细说明 5 个内置扇区（Sector）计算项的计算逻辑、触发条件、临界情况处理。

---

## 综述

扇区计算项通过监听 `SessionSample.current_sector_index` 的变化来检测扇区完成事件。
当 `current_sector_index` 发生变化时，表示车辆刚刚完成了一个扇区并进入了下一个扇区。

### 核心数据源

| 字段 | 类型 | 来源 | 说明 |
|------|------|------|------|
| `current_sector_index` | `i32` | `SessionSample` | 当前所在扇区索引（0=起点/终点直道, 1=扇区1, 2=扇区2） |
| `is_valid_lap` | `i32` | `SessionSample` | 当前圈是否有效（0=无效, 非0=有效） |
| `last_sector_time` | `i32` | `TimingSample` | 最近完成的扇区用时（毫秒） |

### 扇区索引说明

ACC 中扇区索引为 0-based：
- **0**：起点/终点直道（当前圈刚开始，或刚过终点线进入新一圈）
- **1**：通过扇区 1 终点线后进入的区域
- **2**：通过扇区 2 终点线后进入的区域

当一个扇区完成时，`current_sector_index` 会递增（或从 2 回绕到 0 表示新一圈开始）。此时 `last_sector_time` 中保存的就是**刚刚完成的那个扇区**的用时。

### 扇区索引变化示例

```
帧 N:   current_sector_index = 0  （在起点直道）
帧 N+1: current_sector_index = 1  （刚通过扇区1终点线，进入扇区1 → 扇区0已完成）
                                     ↑ last_sector_time = 扇区0的用时

帧 M:   current_sector_index = 1  （在扇区1内）
帧 M+1: current_sector_index = 2  （刚通过扇区2终点线 → 扇区1已完成）
                                     ↑ last_sector_time = 扇区1的用时

帧 K:   current_sector_index = 2  （在扇区2内）
帧 K+1: current_sector_index = 0  （刚通过终点线，新一圈开始 → 扇区2已完成）
                                     ↑ last_sector_time = 扇区2的用时
```

---

## 1. calc:prev_sector_time — 上一个扇区耗时

### 基本信息

| 属性 | 值 |
|------|-----|
| **Item Key** | `calc:prev_sector_time` |
| **描述** | 上一个扇区耗时（ms） |
| **单位** | ms（毫秒） |
| **需要参考圈** | 否 |
| **实现类型** | `RealtimeComputeItem`（逐帧计算） |
| **状态共享** | 与 `calc:prev_sector_number` 共享 `Arc<Mutex<SectorState>>` |

### 计算逻辑

```
每一帧执行：
1. 获取当前帧的 current_sector_index（当前扇区）、is_valid_lap（圈有效性）、last_sector_time（最近扇区用时）
2. 如果 current_sector_index ≠ 历史记录中的 last_seen_sector，且 last_seen_sector ≠ -1（非首帧）：
   a. 说明刚发生扇区切换，上一个扇区已完成
   b. 将 prev_sector_time 设置为 last_sector_time（即刚完成扇区的用时）
   c. 如果 is_valid_lap == 0（当前圈无效）：将 prev_sector_time 覆盖为 -1.0
3. 更新 last_seen_sector = current_sector_index（记录当前扇区位置）
4. 返回当前的 prev_sector_time
```

### 计算条件

| 条件 | 处理 |
|------|------|
| 还没有发生过扇区切换（首帧或一直是同一扇区） | 返回 `-1.0` |
| 发生了扇区切换 + 圈有效（`is_valid_lap ≠ 0`） | 返回刚完成扇区的 `last_sector_time`（ms） |
| 发生了扇区切换 + 圈无效（`is_valid_lap == 0`） | 返回 `-1.0` |

### 临界情况处理

| 场景 | 处理方式 | 返回值 |
|------|----------|--------|
| **首次调用（无历史数据）** | `last_seen_sector = -1`，不满足切换判定条件（`last_seen_sector ≠ -1`） | `-1.0` |
| **同一个扇区内多帧** | `current_sector_index` 不变，不触发切换逻辑 | 保持上一次的 prev_sector_time |
| **跨扇区跳跃（如 0→2，跳过 1）** | 只记录当前帧的 `last_sector_time`，这是刚刚完成的那个扇区的时间 | 依据 `is_valid_lap` 返回 `last_sector_time` 或 `-1.0` |
| **扇区回绕（2→0，新一圈开始）** | 正常触发切换，上一个扇区为 2 | 依据 `is_valid_lap` 返回 `last_sector_time` 或 `-1.0` |
| **圈无效但扇区数据存在** | 切换被检测到，但 `is_valid_lap == 0`，`prev_sector_time` 被设为 `-1.0` | `-1.0` |
| **last_sector_time 为负数或 0** | 不会被过滤——直接使用原始值（仅在 `last_sector_time > 0` 的检查用于 `sector_best`，不用于 `prev_sector_time`） | 原始 `last_sector_time` 值 |
| **Mutex 竞争** | 每次 `compute()` 调用使用 `lock().unwrap()` 获取互斥锁；DashboardService 串行计算各 item，不会发生同一帧内的并发竞争 | — |

---

## 2. calc:prev_sector_number — 上一个扇区编号

### 基本信息

| 属性 | 值 |
|------|-----|
| **Item Key** | `calc:prev_sector_number` |
| **描述** | 上一个扇区编号 |
| **单位** | 无（纯数字） |
| **需要参考圈** | 否 |
| **实现类型** | `RealtimeComputeItem`（逐帧计算） |
| **状态共享** | 与 `calc:prev_sector_time` 共享 `Arc<Mutex<SectorState>>` |

### 计算逻辑

与 `calc:prev_sector_time` **完全相同的扇区切换检测逻辑**。
两者共享同一个 `SectorState`（通过 `Arc<Mutex<SectorState>>` 共享），因此无论在某一帧中哪个 item 先被计算，扇区切换都会被检测并记录一次，两个 item 返回的值始终保持一致。

```
每一帧执行：
1. 获取当前帧的 current_sector_index（当前扇区）、is_valid_lap（圈有效性）、last_sector_time（最近扇区用时）
2. 如果 current_sector_index ≠ 历史记录中的 last_seen_sector，且 last_seen_sector ≠ -1（非首帧）：
   a. 说明刚发生扇区切换，上一个扇区已完成
   b. 将 prev_sector_number 设置为 last_seen_sector（即刚完成的扇区的编号，0-based）
   c. 将 prev_sector_time 也同步设置为 last_sector_time（保持状态一致）
   d. 如果 is_valid_lap == 0：prev_sector_time = -1.0（但 prev_sector_number 不变，仍然报告扇区编号）
3. 更新 last_seen_sector = current_sector_index
4. 返回当前的 prev_sector_number
```

### 计算条件

| 条件 | 处理 |
|------|------|
| 还没有发生过扇区切换 | 返回 `-1.0` |
| 发生了扇区切换 | 返回刚完成扇区的索引（0-based），**不受 `is_valid_lap` 影响** |

> **设计决策**：即使圈无效，`prev_sector_number` 也会报告扇区编号。只有 `prev_sector_time` 在圈无效时被设为 `-1.0`。
> 这样设计是因为扇区编号是导航/定位信息，不应受圈有效性影响。

### 返回值映射

| 刚完成的扇区 | `prev_sector_number` 输出 |
|-------------|--------------------------|
| 扇区 0（起点直道） | `0.0` |
| 扇区 1 | `1.0` |
| 扇区 2 | `2.0` |
| 尚无扇区完成 | `-1.0` |

### 临界情况处理

| 场景 | 处理方式 | 返回值 |
|------|----------|--------|
| **首次调用** | 同 prev_sector_time，`last_seen_sector = -1` 阻止误触发 | `-1.0` |
| **扇区回绕（2→0）** | 切换前 `last_seen_sector = 2`，这是刚完成的扇区 | `2.0` |
| **圈无效** | 扇区编号仍然报告，不受 `is_valid_lap` 影响 | 正常扇区索引 |

---

## 3. calc:sector_best_1 / calc:sector_best_2 / calc:sector_best_3 — 各扇区最佳耗时

### 基本信息

| 属性 | 值 |
|------|-----|
| **Item Key** | `calc:sector_best_1`, `calc:sector_best_2`, `calc:sector_best_3` |
| **描述** | Sector1/2/3 最佳耗时（ms） |
| **单位** | ms（毫秒） |
| **需要参考圈** | 否 |
| **实现类型** | `RealtimeComputeItem`（逐帧计算） |
| **状态共享** | 三个 item 共享同一个 `Arc<Mutex<SectorBestState>>` |
| **固定扇区数** | 3（对于只有 2 个扇区的赛道，`sector_best_3` 始终返回 `-1.0`） |

### 计算逻辑

```
每一帧执行：
1. 获取 current_sector_index、is_valid_lap、last_sector_time
2. 如果 current_sector_index ≠ last_seen_sector（发生扇区切换）：
   a. completed_sector = last_seen_sector（刚完成的扇区索引）
   b. 如果 completed_sector ≥ 0（排除首帧的 -1）：
      - 如果 is_valid_lap ≠ 0（圈有效）
        - 且 last_sector_time > 0（扇区时间有效）
        - 且（best_times[completed_sector] < 0 或 last_sector_time < best_times[completed_sector]）：
          → 更新 best_times[completed_sector] = last_sector_time
   c. 更新 last_seen_sector = current_sector_index
3. 返回 best_times[self.sector_index]（该 item 对应扇区的最佳时间）
```

### 更新条件（必须全部满足才更新最佳时间）

| 条件 | 说明 |
|------|------|
| `current_sector_index ≠ last_seen_sector` | 必须有扇区切换 |
| `last_seen_sector ≥ 0` | 排除首帧（`last_seen_sector = -1`） |
| `is_valid_lap ≠ 0` | 必须是有效圈 |
| `last_sector_time > 0` | 扇区时间必须为正数（排除无效/损坏数据） |
| `best_times[idx] < 0` **或** `last_sector_time < best_times[idx]` | 尚无记录 或 比当前最佳更快 |

### 临界情况处理

| 场景 | 处理方式 |
|------|----------|
| **首帧（last_seen_sector = -1）** | `completed_sector = -1`，但因为 `completed_sector ≥ 0` 检查失败，不会越界访问数组 |
| **扇区回绕（2→0，新一圈开始）** | `completed_sector = 2`，正常更新 `best_times[2]` |
| **圈无效** | `is_valid_lap == 0`，不会更新任意扇区的最佳时间 |
| **扇区时间为 0 或负数** | `last_sector_time > 0` 检查失败，不会更新（防止数据损坏） |
| **较慢的扇区时间** | `last_sector_time` 不小于当前最佳，不会覆盖 |
| **跨越多圈** | 最佳时间持久化在 `SectorBestState` 中，不会因新圈开始而重置。只要 session 不重启，最佳时间会一直保留 |
| **只有 2 个扇区的赛道** | `sector_best_3`（`self.sector_index = 2`）始终返回 `-1.0`，因为扇区 2 永远不会完成 |
| **Mutex 竞争** | 三个 `SectorBestItem` 共享同一个 `Arc<Mutex<SectorBestState>>`，Mutex 保证每次只有一个 item 在读写状态 |
| **数组越界** | `completed_sector < 3` 检查（`completed_sector` 来自 `i32`，转为 `usize` 后始终在合法范围内，且 ACC 最大扇区数为 3） |

### 三个 item 的关系

三个 `SectorBestItem` 实例通过工厂函数 `create_sector_best_items()` 创建，共享同一个 `SectorBestState`：

```
Arc<Mutex<SectorBestState>>
  ├── SectorBestItem { sector_index: 0 } → 返回 best_times[0]（扇区 0 最佳）
  ├── SectorBestItem { sector_index: 1 } → 返回 best_times[1]（扇区 1 最佳）
  └── SectorBestItem { sector_index: 2 } → 返回 best_times[2]（扇区 2 最佳）
```

### 示例：完整的多圈场景

```
初始状态:
  best_times = [-1.0, -1.0, -1.0]

第 1 圈（有效）：
  扇区 0→1: last_sector_time=30000, is_valid_lap=1 → best_times[0] = 30000
  扇区 1→2: last_sector_time=25000, is_valid_lap=1 → best_times[1] = 25000
  扇区 2→0: last_sector_time=20000, is_valid_lap=1 → best_times[2] = 20000
  结果: [-1.0, 25000, 20000] （注意：扇区0在首次调用后变为被"完成"的扇区，但首帧 last_sector_time 可能不是真正的扇区0时间）
  实际上第1帧 last_seen_sector=-1，所以 current_sector_index=0 时不会触发更新
  第2帧 current_sector_index=1，last_seen_sector=0，completed_sector=0，更新 best_times[0]=30000
  结果: [30000, 25000, 20000]

第 2 圈（有效，部分扇区更快）：
  扇区 0→1: last_sector_time=28000, is_valid_lap=1 → 28000 < 30000 → best_times[0] = 28000
  扇区 1→2: last_sector_time=26000, is_valid_lap=1 → 26000 > 25000 → 不更新
  扇区 2→0: last_sector_time=19000, is_valid_lap=1 → 19000 < 20000 → best_times[2] = 19000
  结果: [28000, 25000, 19000]

第 3 圈（无效）：
  扇区 0→1: last_sector_time=24000, is_valid_lap=0 → 不更新（圈无效）
  扇区 1→2: last_sector_time=22000, is_valid_lap=0 → 不更新
  扇区 2→0: last_sector_time=18000, is_valid_lap=0 → 不更新
  结果: [28000, 25000, 19000]（保持不变）
```

---

## 实现架构

### 状态结构体

```rust
// 由 calc:prev_sector_time 和 calc:prev_sector_number 共享
pub struct SectorState {
    pub last_seen_sector: i32,       // 上一帧的 current_sector_index（-1 = 初始）
    pub prev_sector_time: f64,       // 上一个扇区的耗时（ms），-1.0 = 无数据
    pub prev_sector_number: f64,     // 上一个扇区的编号（0-based），-1.0 = 无数据
}

// 由 calc:sector_best_1 / _2 / _3 共享
pub struct SectorBestState {
    pub last_seen_sector: i32,       // 上一帧的 current_sector_index（-1 = 初始）
    pub best_times: [f64; 3],        // 各扇区最佳时间，-1.0 = 尚无数据
}
```

### 工厂函数

```rust
// 创建共享状态的 prev_sector 项对
pub fn create_prev_sector_items() -> (PrevSectorTimeItem, PrevSectorNumberItem)

// 创建共享状态的 3 个 sector_best 项
pub fn create_sector_best_items() -> (SectorBestItem, SectorBestItem, SectorBestItem)
```

### 线程安全

所有扇区计算项通过 `Arc<Mutex<T>>` 共享状态。因为 `DashboardService` 在主循环中**串行**调用每个 item 的 `compute()` 方法，同一帧内不会发生并发竞争。Mutex 主要用于：
1. 跨帧的指针共享（`Arc` 保证生命周期）
2. 将来可能的并发计算场景预留安全边界

---

## 与 Dashboard 的集成

这些内置计算项需要通过 `ComputeRegistry` 注册后才能使用：

```rust
let mut registry = ComputeRegistry::new();

// 注册前扇区项对
let (time_item, number_item) = create_prev_sector_items();
registry.register_calc_realtime(Box::new(time_item)).unwrap();
registry.register_calc_realtime(Box::new(number_item)).unwrap();

// 注册扇区最佳项
let (sb1, sb2, sb3) = create_sector_best_items();
registry.register_calc_realtime(Box::new(sb1)).unwrap();
registry.register_calc_realtime(Box::new(sb2)).unwrap();
registry.register_calc_realtime(Box::new(sb3)).unwrap();

// 订阅（通过 DashboardCommand）
service.subscribe(ItemKey::parse("calc:prev_sector_time").unwrap(), interval, None);
```

注册后，这些 item 会在 `DashboardService::process_frame()` 中按照订阅的间隔自动计算，结果通过 `DataSink::send()` 发送到上游程序。
