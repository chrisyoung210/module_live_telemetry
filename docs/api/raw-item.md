# Raw Item API 文档

## 概述

Raw item 是直接映射到 `TelemetryFrame` 字段的数据项。与 calculated item（需要自定义计算逻辑）不同，raw item 由系统自动解析，**无需注册**即可使用。

内部匹配使用**字符串路径**，格式为 `raw:{子结构体}.{字段名}` 或 `raw:{子结构体}.{字段名}[{索引}]`。

```
用户指定 raw:controls.speed_kmh → 系统匹配 TelemetryFrame.controls.speed_kmh → 返回 f64
```

## 核心 API

### 列出所有 raw item

```rust
use module_live_telemetry::raw_catalog;

let items = raw_catalog::all_raw_items();
// → Vec<RawItemEntry>
```

`RawItemEntry` 结构：

| 字段 | 类型 | 说明 | 示例 |
|---|---|---|---|
| `key` | `ItemKey` | 完整标识键 | `raw:controls.speed_kmh` |
| `description` | `String` | 中文描述 | `"车速"` |
| `unit` | `Option<&str>` | 单位 | `Some("km/h")` |

### 订阅 raw item

```rust
use module_live_telemetry::{
    dashboard::service::DashboardService,
    item_key::ItemKey,
};
use std::time::Duration;

// raw item 无需注册，直接用 ItemKey 订阅
service.subscribe(
    ItemKey::parse("raw:controls.speed_kmh").unwrap(),
    Duration::from_millis(50),
    None,
)?;
```

### 验证字段是否存在

```rust
use module_live_telemetry::writer::TelemetryFrame;

if TelemetryFrame::is_raw_field("controls.speed_kmh") {
    // 字段有效
}
```

## 命名约定

### 标量字段

```
raw:{子结构体}.{字段名}
```

| 键 | 说明 |
|---|---|
| `raw:controls.speed_kmh` | 车速 |
| `raw:controls.rpms` | 发动机转速 |
| `raw:session.completed_laps` | 已完成圈数 |
| `raw:environment.air_temp` | 气温 |

### 数组字段

```
raw:{子结构体}.{字段名}[{索引}]
```

| 键 | 说明 |
|---|---|
| `raw:motion.velocity[0]` | 世界速度 X 轴分量 |
| `raw:motion.velocity[1]` | 世界速度 Y 轴分量 |
| `raw:tyres.tyre_temp[2]` | 轮胎表面温度（后左） |
| `raw:car_state.car_damage[0]` | 车辆损伤（前部） |

数组索引的含义因字段而异：

| 数组大小 | 含义 |
|---|---|
| `[2]` | 前/后轴 |
| `[3]` | X/Y/Z 轴 |
| `[4]` | 前左/前右/后左/后右 (FL/FR/RL/RR) |
| `[5]` | 前部/后部/左侧/右侧/中部 |
| `[12]` | 4轮 × 3轴 |

### 顶层字段

```
raw:{字段名}
```

| 键 | 说明 |
|---|---|
| `raw:sample_tick` | 采样序号 |
| `raw:timestamp_ns` | 时间戳（纳秒） |

## 字段分组总览

所有 raw item 按子结构体分组，共 ~200 个字段：

| 子结构体 | 中文名 | 字段数 | 典型字段 |
|---|---|---|---|
| `controls` | 车辆操控 | 12 | speed_kmh, gas, brake, clutch, steer_angle, gear, rpms, fuel |
| `motion` | 运动数据 | 9 | velocity[0-2], acc_g[0-2], heading, pitch, roll |
| `tyres` | 轮胎数据 | 27 | tyre_temp[0-3], wheel_slip[0-3], brake_temp[0-3], tyre_wear[0-3] |
| `powertrain` | 动力总成 | 23 | turbo_boost, kers_charge, drs, tc, abs, water_temp |
| `session` | 比赛会话 | 31 | position, completed_laps, normalized_car_position, is_in_pit |
| `timing` | 计时数据 | 15 | i_current_time, i_best_time, i_delta_lap_time, used_fuel |
| `car_state` | 车辆状态 | 40 | brake_bias, cg_height, car_damage[0-4], ride_height[0-1] |
| `environment` | 环境数据 | 11 | air_temp, road_temp, wind_speed, rain_intensity, surface_grip |
| `other_cars` | 其他车辆 | 4 | active_cars, player_car_id |

## 完整示例

```rust
use module_live_telemetry::{
    compute::ComputeRegistry,
    dashboard::{service::DashboardService, sink::ChannelSink},
    item_key::ItemKey,
    raw_catalog,
};
use std::time::Duration;
use crossbeam_channel::bounded;

// 1. 列出所有 raw item
println!("=== 可用 Raw Item ===");
for item in raw_catalog::all_raw_items() {
    let unit = item.unit.unwrap_or("");
    if !unit.is_empty() {
        println!("  {} — {} [{}]", item.key, item.description, unit);
    } else {
        println!("  {} — {}", item.key, item.description);
    }
}

// 2. 创建 DashboardService 并订阅
let registry = ComputeRegistry::new();
let (tx, rx) = bounded(10);
let sink = ChannelSink::new(tx);
let mut service = DashboardService::new(registry, Box::new(sink));

// 订阅几个 raw item
let items = [
    "raw:controls.speed_kmh",
    "raw:controls.rpms",
    "raw:controls.gear",
    "raw:session.completed_laps",
];

for item_name in &items {
    service.subscribe(
        ItemKey::parse(item_name).unwrap(),
        Duration::from_millis(50),
        None,
    ).unwrap();
}

// 3. 运行（service.run(frame_receiver)）
```

## Item 类型对比

| | raw | calc | system |
|---|---|---|---|
| **是否需要注册** | 否 | 是 | 未来 |
| **命名格式** | `raw:{子结构体}.{字段}` | `calc:{名称}` | `system:{名称}` |
| **数据来源** | TelemetryFrame 字段 | 自定义计算 | 系统 API |
| **匹配方式** | 字符串路径 | 字符串名称 | （未实现） |
| **数量** | ~200 | 按注册 | 未来 |
