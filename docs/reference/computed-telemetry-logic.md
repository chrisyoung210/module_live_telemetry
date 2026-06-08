# Live Recording 计算遥测项逻辑说明

本文档说明当前代码中实际存在的遥测数据计算逻辑。所有计算均在 `src/bin/acc-live-telemetry.rs` 中完成。

---

## 1. 圈速边界检测（Lap Boundary Detection）

### 1.1 检测逻辑

**代码位置**: `src/bin/acc-live-telemetry.rs` - `laps_command()` / `build_lap_index_command()` / `append_lap_index_to_file()`

**输入数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `normalized_car_position` | `f32` | Session cluster - 赛道归一化位置 (0.0~1.0) |

**检测条件**:
```
上一帧 normalized_car_position > 0.8 且 当前帧 normalized_car_position < 0.2
```

**说明**:
- 当车辆通过起点/终点线时，`normalized_car_position` 会从接近 1.0 跳变到接近 0.0
- 阈值 0.8 / 0.2 用于鲁棒检测，避免误触发
- 检测到的位置即为新一圈的开始

---

## 2. 圈速有效性判断（Lap Validity）

### 2.1 判断逻辑

**代码位置**: `src/bin/acc-live-telemetry.rs` - `laps_command()` / `build_lap_index_command()`

**输入数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `is_valid_lap` | `i32` | Session cluster - 是否为有效圈 (0/1) |

**判断规则**:
- **Out lap**（第 0 圈）: 始终标记为无效
- **其他圈**: 检查圈末最后 3 个样本的 `is_valid_lap` 标志
  - 如果任意一个样本的 `is_valid_lap == 0`，则该圈无效
  - 否则该圈有效

**说明**:
- 检查最后 3 个样本而非仅最后一个，是因为 `normalized_car_position` 更新比 `is_valid_lap` 标志有 2-3 tick 的滞后
- 对于最后一圈（可能未完成），同样使用最后 3 个样本判断

---

## 3. 计时数据匹配（Timing Data Matching）

### 3.1 匹配逻辑

**代码位置**: `src/bin/acc-live-telemetry.rs` - `laps_command()`

**输入数据**:
| 字段 | 类型 | 来源 |
|------|------|------|
| `sample_tick` | `u64` | Session / Timing cluster - 样本序号 |
| `i_last_time` | `i32` | Timing cluster - 上一圈时间（毫秒） |
| `i_best_time` | `i32` | Timing cluster - 最佳圈时间（毫秒） |

**匹配规则**:
1. 在圈速边界点（ crossing 点），查找相同或最近的 `sample_tick` 的 timing 样本
2. 使用二分搜索在 timing 数据中定位
3. 如果找不到精确匹配，取该圈范围内的最后一个 timing 样本

**有效性过滤**:
```
i_last_time > 0 且 i_last_time < 2_000_000 毫秒（约 33 分钟）
i_best_time > 0 且 i_best_time < 2_000_000 毫秒
```

**时间格式化**:
```
mm:ss.mmm = ms / 60000 : (ms % 60000) / 1000 : ms % 1000
```

---

## 4. 字段导出（Field Export）

### 4.1 字段映射

**代码位置**: `src/bin/acc-live-telemetry.rs` - `export_lap_field_command()`

**支持的字段来源**:

**Session 字段**:
- `status`, `session`, `sessionIndex`, `completedLaps`, `position`
- `sessionTimeLeft`, `numberOfLaps`, `currentSectorIndex`, `normalizedCarPosition`
- `isInPit`, `isInPitLane`, `mandatoryPitDone`, `missingMandatoryPits`
- `penaltyTime`, `penaltyType`, `clock`, `replayTimeMultiplier`, `isValidLap`
- `globalYellow`, `globalYellow1`, `globalYellow2`, `globalYellow3`
- `globalWhite`, `globalGreen`, `globalChequered`, `globalRed`
- `gapAheadOrTailValue`, `flag`, `gapBehind`

**Timing 字段**:
- `iCurrentTime`, `iLastTime`, `iBestTime`, `iSplit`, `lastSectorTime`
- `iDeltaLapTime`, `isDeltaPositive`, `iEstimatedLapTime`
- `fuelEstimatedLaps`, `fuelXLap`, `usedFuel`, `distanceTraveled`

**Controls 字段**:
- `speedKmh`, `gas`, `brake`, `clutch`, `steerAngle`, `gear`, `rpms`, `fuel`
- `physicsPacketId`, `graphicsPacketId`

**匹配逻辑**:
- Session 和 Timing 数据按 `sample_tick` 精确匹配
- Controls 数据按 `sample_tick` 查找，找不到时输出 `"?"`
- 不认识的字段名输出 `"?"`
