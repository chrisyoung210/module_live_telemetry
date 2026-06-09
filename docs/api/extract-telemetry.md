# 圈遥测数据提取 API 参考

将 `.acctlm` 文件中指定 raw 字段的遥测数据按圈组织提取，供主模块（m1）在数据分析、可视化场景中使用。

模块提供两个 API：

| API | 用途 |
|---|---|
| `extract_lap_telemetry` | 提取**全部**圈的数据 |
| `extract_laps_telemetry` | 提取**指定圈号列表**对应圈的数据 |

---

## 1. extract_lap_telemetry — 提取全部圈

### 1.1 快速开始

```rust
use std::collections::HashSet;
use module_live_telemetry::{extract_lap_telemetry, item_key::ItemKey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 指定要提取的字段
    let mut keys = HashSet::new();
    keys.insert(ItemKey::parse("raw:controls.speed_kmh").unwrap());
    keys.insert(ItemKey::parse("raw:controls.brake").unwrap());
    keys.insert(ItemKey::parse("raw:session.normalized_car_position").unwrap());

    // 2. 调用 API
    let laps = extract_lap_telemetry("recording_20260606.acctlm", &keys)?;

    // 3. 使用结果
    println!("共 {} 圈", laps.len());

    for (lap_idx, lap_data) in laps.iter().enumerate() {
        let speed = lap_data.get("raw:controls.speed_kmh").unwrap();
        let brake = lap_data.get("raw:controls.brake").unwrap();

        println!(
            "Lap {}: {} frames, max speed {:.1} km/h, avg brake {:.3}",
            lap_idx,
            speed.len(),
            speed.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            brake.iter().sum::<f64>() / brake.len().max(1) as f64,
        );
    }

    Ok(())
}
```

### 1.2 函数签名

```rust
pub fn extract_lap_telemetry(
    path: impl AsRef<Path>,
    keys: &HashSet<ItemKey>,
) -> TelemetryResult<Vec<HashMap<String, Vec<f64>>>>
```

**参数**：

| 参数 | 类型 | 说明 |
|---|---|---|
| `path` | `impl AsRef<Path>` | `.acctlm` 文件路径。支持 `&str`、`String`、`Path`、`PathBuf`。 |
| `keys` | `&HashSet<ItemKey>` | 要提取的 raw 字段集合，key 格式为 `raw:cluster.field`，与 [`all_raw_items()`](raw-item.md#列出所有-raw-item) 返回格式一致。 |

**返回值**：

- `Ok(Vec<HashMap<String, Vec<f64>>>)` — 提取成功。
- `Err(TelemetryError)` — 文件不合法、无法读取，或字段 key 不合法。

---

## 2. extract_laps_telemetry — 提取指定圈

### 2.1 快速开始

```rust
use std::collections::HashSet;
use module_live_telemetry::{extract_laps_telemetry, item_key::ItemKey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 指定要提取的字段
    let mut keys = HashSet::new();
    keys.insert(ItemKey::parse("raw:controls.speed_kmh").unwrap());
    keys.insert(ItemKey::parse("raw:controls.brake").unwrap());

    // 2. 指定只提取第 1、5、8 圈
    let lap_numbers = &[1, 5, 8];

    // 3. 调用 API
    let laps = extract_laps_telemetry("recording_20260606.acctlm", &keys, lap_numbers)?;

    // 结果只包含 3 个元素，顺序与 lap_numbers 一致
    println!("共提取 {} 圈", laps.len()); // → 3

    for (i, lap_data) in laps.iter().enumerate() {
        let speed = lap_data.get("raw:controls.speed_kmh").unwrap();
        let brake = lap_data.get("raw:controls.brake").unwrap();

        println!(
            "Lap {} (请求的第 {} 圈): {} frames",
            lap_numbers[i],  // 原始圈号
            lap_numbers[i],
            speed.len(),
        );
    }

    Ok(())
}
```

### 2.2 函数签名

```rust
pub fn extract_laps_telemetry(
    path: impl AsRef<Path>,
    keys: &HashSet<ItemKey>,
    lap_numbers: &[usize],
) -> TelemetryResult<Vec<HashMap<String, Vec<f64>>>>
```

### 2.3 参数

| 参数 | 类型 | 说明 |
|---|---|---|
| `path` | `impl AsRef<Path>` | `.acctlm` 文件路径。支持 `&str`、`String`、`Path`、`PathBuf`。 |
| `keys` | `&HashSet<ItemKey>` | 要提取的 raw 字段集合，同 `extract_lap_telemetry`。 |
| `lap_numbers` | `&[usize]` | 要提取的圈号列表（0-based）。结果按此列表顺序返回，每个元素对应一个圈。 |

**返回值**：

- `Ok(Vec<HashMap<String, Vec<f64>>>)` — 提取成功。返回的 `Vec` 长度等于 `lap_numbers` 的长度，结果顺序与 `lap_numbers` 一一对应。
- `Err(TelemetryError::InvalidArgument)` — `lap_numbers` 中包含不存在的圈号（如文件只有 5 圈却请求了第 10 圈）。

---

## 3. 返回值结构

两个 API 的返回值结构相同（仅 `extract_laps_telemetry` 按 `lap_numbers` 筛选了特定圈）：

```
Vec<HashMap<String, Vec<f64>>>
 │
 └─[0] Lap 0 (out-lap)
 │   ├─ "raw:controls.speed_kmh" → [0.0, 0.5, 1.2, ...]
 │   ├─ "raw:controls.brake"     → [0.0, 0.0, 0.3, ...]
 │   └─ "raw:session.normalized_car_position" → [0.01, 0.05, ...]
 │
 └─[1] Lap 1 (first flying lap)
 │   ├─ "raw:controls.speed_kmh" → [185.3, 187.1, 184.9, ...]
 │   └─ ...
 │
 └─[2] Lap 2
     └─ ...
```

| 层级 | 类型 | 说明 |
|---|---|---|
| 外层 `Vec` | `Vec<T>` | **`extract_lap_telemetry`**：按圈号排序，`index=0` 为第 0 圈（out-lap）。**`extract_laps_telemetry`**：按 `lap_numbers` 参数顺序排列。 |
| 中层 `HashMap` | `HashMap<String, Vec<f64>>` | key 为传入的完整字段标识串（如 `"raw:controls.speed_kmh"`），value 为这一圈内该字段在所有帧中的值序列。 |
| 内层 `Vec<f64>` | `Vec<f64>` | 该字段在当前圈中按时间顺序排列的逐帧值。长度等于该字段在当前圈中的采样帧数。 |

**注意**：不同 cluster 可能有不同的采样频率，导致同一圈内不同字段的 `Vec<f64>` 长度不同（例如 `controls` 可能 120 Hz 而 `session` 只有 60 Hz）。圈边界基于 `session` cluster 的 `normalized_car_position` 检测，因此 `session` 类字段的值数量等于该圈的 session 帧数。

---

## 4. 圈边界检测

圈边界通过 `session` cluster 的 `normalized_car_position` 字段检测：

- `normalized_car_position` 是 0.0~1.0 的值，表示车辆在赛道上的归一化位置。
- 当该值从 `> 0.8` 跳变到 `< 0.2` 时，判定为一次起/终点线穿越。
- 最后一圈如果未完成（录制中断），仍会作为一个不完整圈返回。

此算法与 [`parse_acctlm_file`](import.md) 内部的 `aggregate_laps` 完全一致。

---

## 5. 验证规则

两个 API 在读文件之前会先校验所有传入的 `ItemKey`：

| 校验项 | 失败原因 | 错误类型 |
|---|---|---|
| `ItemKey.item_type` 必须为 `Raw` | 传入了 `calc:*` 或 `system:*` 类型 | `TelemetryError::InvalidArgument` |
| 子结构体名称合法 | `controls`、`session`、`motion`、`tyres`、`powertrain`、`timing`、`car_state`、`environment`、`other_cars` 之外 | `TelemetryError::InvalidArgument` |
| 字段名在子结构体中存在 | 拼写错误或不存在于该 cluster 中 | `TelemetryError::InvalidArgument` |
| 顶级字段名合法 | `sample_tick`、`timestamp_ns` 之外 | `TelemetryError::InvalidArgument` |

此外，`extract_laps_telemetry` 额外校验：

| 校验项 | 失败原因 | 错误类型 |
|---|---|---|
| `lap_numbers` 中的圈号存在 | 圈号超出文件实际圈数范围 | `TelemetryError::InvalidArgument` |

文件读取阶段的校验规则与 [`parse_acctlm_file`](import.md#3-验证规则) 相同。

---

## 6. 支持的字段格式

字段 key 格式为 `raw:{cluster}.{field}`，与 [`all_raw_items()`](raw-item.md#列出所有-raw-item) 完全一致：

```
raw:controls.speed_kmh          → controls cluster 的 speed_kmh 字段
raw:controls.brake              → controls cluster 的 brake 字段
raw:session.normalized_car_position → session cluster 的 normalized_car_position 字段
raw:motion.velocity[0]          → motion cluster 的 velocity 数组第 0 项
raw:sample_tick                 → 顶级字段 sample_tick
raw:timestamp_ns                → 顶级字段 timestamp_ns
```

**跨 cluster 支持**：可以在一次调用中同时提取来自不同 cluster 的字段（如 `controls.speed_kmh` 和 `session.normalized_car_position`），系统会按需只读取涉及到的 cluster 数据。

完整字段列表参见 [Raw Item API 文档](raw-item.md)。

---

## 7. 边界情况

| 场景 | `extract_lap_telemetry` | `extract_laps_telemetry` |
|---|---|---|
| `keys` 为空集合 | 返回 `Ok(Vec::new())` | 返回 `Ok(Vec::new())` |
| `lap_numbers` 为空 | — | 返回 `Ok(Vec::new())` |
| 文件无 session 数据 | 返回 `Ok(Vec::new())` | 返回 `Ok(Vec::new())` |
| 文件不存在 / 无法读取 | 返回 `Err(TelemetryError::Io)` | 返回 `Err(TelemetryError::Io)` |
| 文件格式不合法 | 返回 `Err(TelemetryError::InvalidFormat)` | 返回 `Err(TelemetryError::InvalidFormat)` |
| 某字段在某圈无数据 | 该字段在该圈的 `Vec<f64>` 为空 | 该字段在该圈的 `Vec<f64>` 为空 |
| 传入不存在的字段名 | 返回 `Err(TelemetryError::InvalidArgument)` | 返回 `Err(TelemetryError::InvalidArgument)` |
| 请求的圈号不存在 | — | 返回 `Err(TelemetryError::InvalidArgument)` |

---

## 8. 相关 API

- [acctlm 文件导入 (`parse_acctlm_file`)](import.md) — 解析文件摘要信息（赛道、车型、每圈圈时）
- [Raw Item API](raw-item.md) — 列出所有可用 raw 字段
- [RecordingController API](recording.md) — 实时录制控制
