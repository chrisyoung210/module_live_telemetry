# Live Recording 计算遥测项逻辑说明

本文档详细说明 ACC Coach 在 live recording 过程中，通过二次计算获取遥测数据的逻辑。所有计算均基于原始（raw）遥测数据，在实时采集或导入回放时完成。

---

## 1. 距离积分（Distance Integration）

### 1.1 总距离积分（`distance_m`）

**代码位置**: `src/live/shared_memory.rs` - `AccLiveReader::update_distance()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `speed_kmh` | `f64` | ACC Physics Shared Memory - 车辆当前速度（km/h） |
| `timestamp_ms` | `u64` | 系统启动后的累计毫秒数 |

**计算逻辑**:

当 ACC 的 Graphics Shared Memory 未提供 `distance_traveled_m` 时，系统通过速度积分计算累计行驶距离。

```
dt_s = (current_timestamp_ms - previous_timestamp_ms) / 1000.0
      （限制条件：0.0 < dt_s < 1.0，超出范围则 dt_s = 0.0）

speed_ms_current = speed_kmh / 3.6
speed_ms_previous = previous_speed_kmh / 3.6

distance_m += ((speed_ms_previous + speed_ms_current) / 2.0) * dt_s
```

**算法说明**:
- 采用**梯形法**（Trapezoidal Rule）进行数值积分，使用前后两个采样点的平均速度乘以时间间隔
- 时间间隔限制在 0~1 秒之间，防止异常时间跳跃导致距离突变
- 每次计算后更新 `previous_timestamp_ms` 和 `previous_speed_kmh`

---

### 1.2 单圈距离积分（`speed_integrated_lap_distance_m`）

**代码位置**: `src/live/shared_memory.rs` - `AccLiveReader::update_lap_distances()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `speed_kmh` | `f64` | ACC Physics Shared Memory |
| `timestamp_ms` | `u64` | 系统累计时间 |
| `completed_laps` | `Option<i32>` | ACC Graphics Shared Memory - 已完成圈数 |
| `current_lap_time_ms` | `Option<i32>` | ACC Graphics Shared Memory - 当前单圈时间 |
| `normalized_car_position` | `Option<f64>` | ACC Graphics Shared Memory - 赛道归一化位置 (0.0~1.0) |

**圈切换检测条件**（满足任一即重置积分）:
1. **圈数变化**: `completed_laps` 发生变化且不为 None
2. **时间重置**: 上一帧 `current_lap_time_ms` - 当前帧 `current_lap_time_ms` > 30,000ms（说明跨过了终点线）
3. **位置环绕**: 上一帧 `normalized_car_position` > 0.8 且当前帧 < 0.2（说明通过了起点/终点线）

**计算逻辑**:

圈切换时重置：
```
speed_integrated_lap_distance_m = 0.0
lap_start_distance_m = current_total_distance_m
```

正常积分时：
```
dt_s = (current_timestamp_ms - previous_speed_integrated_timestamp_ms) / 1000.0
      （限制条件：0.0 < dt_s < 1.0）

prev_speed_ms = previous_speed_integrated_kmh / 3.6
current_speed_ms = speed_kmh / 3.6

speed_integrated_lap_distance_m += ((prev_speed_ms + current_speed_ms) / 2.0) * dt_s
```

---

### 1.3 归一化单圈距离（`current_lap_distance_m` / `normalized_m`）

**代码位置**: `src/live/shared_memory.rs` - `AccLiveReader::update_lap_distances()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `normalized_car_position` | `Option<f64>` | ACC Graphics Shared Memory (0.0~1.0) |
| `track_lap_distance_m` | `Option<f64>` | 用户配置或默认赛道长度 |
| `distance_m` (累计) | `f64` | 上述总距离积分结果 |
| `lap_start_distance_m` | `Option<f64>` | 本圈开始时的累计距离 |

**计算逻辑**:

优先使用赛道归一化位置：
```
if normalized_car_position 存在且为有限值:
    normalized_lap_distance_m = normalized_car_position.clamp(0.0, 1.0) * track_lap_distance_m
else:
    // Fallback：使用累计距离差
    if lap_start_distance_m 存在:
        cumulative_lap_distance_m = (distance_m - lap_start_distance_m).max(0.0)
        normalized_lap_distance_m = cumulative_lap_distance_m
```

**说明**:
- `track_lap_distance_m` 优先使用用户配置的赛道长度，否则使用默认值 `DEFAULT_LIVE_LAP_DISTANCE_M = 5,793.0` 米
- 该值同时被更新到 `self.current_lap_distance_m`，供后续 delta 计算使用

---

## 2. Delta 计算（Lap Delta Calculation）

Delta 计算用于比较当前驾驶圈与参考圈（最佳圈或 Session 最佳圈）的时间差异。

### 2.1 参考圈数据结构

**代码位置**: `src/live/shared_memory.rs` - `LiveBestLapReference`

参考圈由一系列 `BestLapPoint` 组成：
```rust
struct BestLapPoint {
    distance_m: f64,    // 赛道位置（米）
    elapsed_ms: f64,    // 到达该位置的累计时间（毫秒）
}
```

### 2.2 原始 Delta 计算（`delta_time_ms`）

**代码位置**: `src/live/shared_memory.rs` - `LiveBestLapReference::delta_time_ms()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `current_lap_time_ms` | `Option<i32>` | ACC Graphics Shared Memory - 当前单圈已用时间 |
| `normalized_car_position` | `Option<f64>` | ACC Graphics Shared Memory - 赛道归一化位置 |
| `fallback_distance_m` | `f64` | `current_lap_distance_m`（上述计算结果） |

**计算步骤**:

**Step 1: 获取当前时间**
```
current_ms = current_lap_time_ms
            （过滤条件：-2,147,483,647 < current_ms < 2,147,483,647，否则为 null）
```

**Step 2: 确定当前赛道位置（米）**
```
if fallback_distance_m 存在且为有限值且 > 0.0:
    distance_m = fallback_distance_m.clamp(0.0, total_distance_m)
else if normalized_car_position 存在且为有限值:
    distance_m = normalized_car_position.clamp(0.0, 1.0) * total_distance_m
else:
    return null
```

**Step 3: 插值获取参考圈在该位置的时间**
```
在参考圈的 points 数组中找到包含 distance_m 的两个相邻点 (a, b)

if span = b.distance_m - a.distance_m < epsilon:
    reference_ms = a.elapsed_ms
else:
    ratio = ((distance_m - a.distance_m) / span).clamp(0.0, 1.0)
    reference_ms = a.elapsed_ms + (b.elapsed_ms - a.elapsed_ms) * ratio
```

**Step 4: 计算 Delta**
```
raw_delta_ms = current_ms - reference_ms
```

**结果解读**:
- `raw_delta_ms > 0`: 当前比参考圈**慢**
- `raw_delta_ms < 0`: 当前比参考圈**快**

---

### 2.3 校准后 Delta（`corrected_ms`）

**代码位置**: `src/live/shared_memory.rs` - `BestLapDeltaCalibrator::correct()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `raw_delta_ms` | `f64` | 上述 `delta_time_ms()` 计算结果 |
| `completed_laps` | `Option<i32>` | ACC Graphics Shared Memory |
| `current_lap_time_ms` | `Option<i32>` | ACC Graphics Shared Memory |
| `progress` | `f64` | 当前赛道进度 (0.0~1.0) |

**计算逻辑**:

**起跑线校准**:
当满足以下条件时，记录起跑偏移量：
```
if current_lap_time_ms <= 500ms 且 progress <= 0.03:
    start_offset_ms = raw_delta_ms.clamp(-500.0, 500.0)
```

**校准公式**:
```
corrected_ms = raw_delta_ms - start_offset_ms * start_offset_weight(progress)
```

其中 `start_offset_weight(progress)` 是一个权重函数，在起跑区域（progress 接近 0）时权重最大，随着 progress 增加逐渐衰减到 0。

**说明**:
- 起跑线校准用于消除起跑时刻计时不同步导致的 delta 偏差
- 每次新圈开始时自动重置校准器

---

### 2.4 ACC 原生 Delta 可用性判断

**代码位置**: `src/live/shared_memory.rs` - `LiveBestLapReference::acc_best_lap_time_matches()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `best_lap_time_ms` (ACC) | `Option<i32>` | ACC Graphics Shared Memory |
| `lap_time_ms` (Reference) | `Option<u64>` | 参考圈总时间 |

**判断逻辑**:
```
acc_delta_usable = |reference_lap_time_ms - acc_best_lap_time_ms| <= 25.0 ms
```

只有当 ACC 报告的最佳圈时间与 ACC Coach 的参考圈时间在 25ms 内匹配时，ACC 原生的 `deltaLapTimeMs` 才被认为是可对比的。

---

### 2.5 Best-Lap Delta 汇总输出

**代码位置**: `src/live/shared_memory.rs` - `AccLiveReader::best_lap_delta_time_ms()`

最终生成的 `BestLapDeltaValue` 包含以下计算字段：

| 字段 | 计算来源 |
|------|---------|
| `corrected_ms` | `BestLapDeltaCalibrator::correct(raw_delta_ms, ...)` |
| `raw_ms` | `LiveBestLapReference::delta_time_ms()` 原始结果 |
| `start_offset_ms` | 校准器记录的起跑偏移量 |
| `reference_time_ms` | 参考圈在当前位置的已用时间（插值结果） |
| `progress` | `distance_m / total_distance_m` (0.0~1.0) |
| `reference_lap_time_ms` | 参考圈总时间 |
| `acc_delta_ms` | ACC 原生 delta（仅在可用时） |
| `acc_delta_usable` | 上述匹配判断结果 |

---

## 3. 预测圈速（Predicted Lap Time）

### 3.1 基于最佳圈的预测

**代码位置**: `src/live/shared_memory.rs` - `predicted_lap_time_by_best()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `reference_lap_time_ms` | `Option<u64>` | 最佳参考圈总时间 |
| `corrected_ms` | `Option<f64>` | 校准后的 delta 时间 |

**计算逻辑**:
```
if reference_lap_time_ms 为 null 或 corrected_ms 为 null:
    return null

predicted_lap_time_by_best = (reference_lap_time_ms + corrected_ms).max(0.0)
```

**公式说明**:
```
预测圈速 = 参考圈总时间 + 当前与参考圈的时差
```

- 如果当前比参考圈快（`corrected_ms < 0`），预测圈速将小于参考圈时间
- 如果当前比参考圈慢（`corrected_ms > 0`），预测圈速将大于参考圈时间

---

### 3.2 基于 Session 最佳圈的预测

**代码位置**: `src/live/shared_memory.rs` - `predicted_lap_time_by_session()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `reference_lap_time_ms` | `Option<u64>` | Session 最佳圈总时间 |
| `corrected_ms` | `Option<f64>` | 与 Session 最佳圈的校准后 delta |

**计算逻辑**: 与 `predicted_lap_time_by_best` 完全相同，只是参考圈来源不同（Session 最佳圈 vs 历史最佳圈）。

```
predicted_lap_time_by_session = (reference_lap_time_ms + corrected_ms).max(0.0)
```

---

## 4. 距离估算（Distance Estimation）

### 4.1 导入时的距离回退计算

**代码位置**: `src/recording/import.rs` - `telemetry_point_from_state()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `raw_distance_m` | `Option<f64>` | 录制文件中的 `distanceM` 字段 |
| `normalized_car_position` | `Option<f64>` | 录制文件中的 `normalizedCarPosition` 字段 (0.0~1.0) |

**计算逻辑**:
```
if raw_distance_m 存在:
    distance_m = raw_distance_m
else if normalized_car_position 存在:
    distance_m = normalized_car_position.clamp(0.0, 1.0) * 10,000.0
else:
    distance_m = 0.0
```

**说明**:
- 旧版本录制文件可能没有 `distanceM` 字段，此时使用 `normalizedCarPosition` 进行估算
- 假设赛道总长度为 10,000 米（固定值），将归一化位置转换为米
- 这是一个**粗略估算**，精度取决于实际赛道长度与 10,000 米的接近程度

---

## 5. 编码转换（Gear Encoding Conversion）

### 5.1 旧版本录制文件齿轮编码转换

**代码位置**: `src/recording/import.rs` - `update_state()`

**输入 Raw 数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `gear` | `i64` | 录制文件中的 `gear` 字段 |
| `recording_version` | `u32` | 录制文件版本号（metadata 中） |

**转换规则**:

**v1 格式（旧版，0-indexed）**:
```
recording_version <= 1:
    if raw_gear == -1:
        gear = 0        // 后退/空挡（无法区分，统一映射为空挡）
    else if raw_gear >= 0:
        gear = raw_gear + 1   // 0→1挡, 1→2挡, 2→3挡...
    else:
        gear = raw_gear       // 保持其他异常值
```

**v2+ 格式（新版，ACC native）**:
```
recording_version >= 2:
    gear = raw_gear    // 直接使用 ACC 原生编码
```

**ACC 原生齿轮编码**:
| 编码值 | 含义 |
|--------|------|
| -1 | 后退挡 (Reverse) |
| 0 | 空挡 (Neutral) |
| 1 | 1挡 |
| 2 | 2挡 |
| ... | ... |

**说明**:
- v1 格式中，-1 同时表示后退挡和空挡，导入时无法区分，统一映射为 0（空挡）
- v2+ 格式保留了 ACC 原生编码，可以正确区分后退挡和空挡

---

## 6. 其他计算项速查

### 6.1 方向盘角度转换

**代码位置**: `src/live/shared_memory.rs` - `SharedMemoryReader::read_raw_frame()`

**输入**: `ps.steer_angle` (ACC Physics, -1.0~1.0 归一化比例)

**计算**:
```
steering_deg = steer_angle * 225.0
```

说明：ACC 存储的是方向盘归一化比例，乘以 225°（半锁止角度，假设总锁止角度 450°）得到实际角度。

---

### 6.2 轮速转换

**代码位置**: `src/recording/import.rs` - `set_wheel_speed()`

**输入**: `wheelAngularSpeed*` (弧度/秒)

**计算**:
```
wheel_speed_kmh = wheel_angular_speed * 3.6
```

---

### 6.3 简单归一化/限制

**代码位置**: `src/live/shared_memory.rs` - `LiveFrame` 构建

| 字段 | 计算 |
|------|------|
| `brake_pct` | `brake.clamp(0.0, 1.0)` |
| `throttle_pct` | `throttle.clamp(0.0, 1.0)` |
| `gear` | `normalize_acc_gear(raw.gear)` (限制在 i8 范围) |
| `rpm` | `rpm.max(0.0) as u32` |

---

## 附录：计算数据流图

```
ACC Shared Memory (Physics + Graphics)
    │
    ├── Physics: speed_kmh, steer_angle, brake, throttle, rpm, gear, wheel_angular_speed...
    └── Graphics: distance_traveled_m, completed_laps, current_lap_time_ms, normalized_car_position, best_lap_time_ms...
    │
    ▼
SharedMemoryReader::read_raw_frame()
    │
    ├── steering_deg = steer_angle * 225.0          [角度转换]
    ├── distance_m = distance_traveled_m (or 速度积分) [距离积分]
    └── 其他字段直接透传
    │
    ▼
AccLiveReader::read_frame()
    │
    ├── update_distance()                           [总距离积分]
    ├── update_lap_distances()                      [单圈距离 + 归一化]
    │   ├── speed_integrated_lap_distance_m         [单圈速度积分]
    │   └── current_lap_distance_m                  [归一化位置 × 赛道长度]
    ├── best_lap_delta_time_ms()                    [Best-Lap Delta]
    │   ├── delta_time_ms()                         [插值对比]
    │   └── BestLapDeltaCalibrator::correct()       [起跑线校准]
    ├── session_lap_delta_time_ms()                 [Session Delta]
    ├── predicted_lap_time_by_best()                [预测圈速]
    └── predicted_lap_time_by_session()             [预测圈速]
    │
    ▼
LiveFrame (包含原始 + 计算字段)
    │
    ▼
LiveRecorder → JSONL 文件 / Dashboard 输出
```

---

## 附录：文件索引

| 计算类型 | 主要代码文件 | 关键函数/结构体 |
|---------|------------|----------------|
| 距离积分 | `src/live/shared_memory.rs` | `AccLiveReader::update_distance()`, `update_lap_distances()` |
| Delta 计算 | `src/live/shared_memory.rs` | `LiveBestLapReference::delta_time_ms()`, `diagnostic()`, `BestLapDeltaCalibrator::correct()` |
| 预测圈速 | `src/live/shared_memory.rs` | `predicted_lap_time_by_best()`, `predicted_lap_time_by_session()` |
| 距离估算 | `src/recording/import.rs` | `telemetry_point_from_state()` |
| 编码转换 | `src/recording/import.rs` | `update_state()` |
| 角度转换 | `src/live/shared_memory.rs` | `SharedMemoryReader::read_raw_frame()` |
| 轮速转换 | `src/recording/import.rs` | `set_wheel_speed()` |
