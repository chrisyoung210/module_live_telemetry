# Calculated Item API

> **相关文档**: [recording.md](recording.md) · [raw-item.md](raw-item.md) · [sector-calculated-items.md](../reference/sector-calculated-items.md)

## 概述

Calculated item 是基于遥测数据**自定义计算逻辑**的扩展点。与 raw item（直接从 `TelemetryFrame` 读字段）不同，calculated item 允许你实现任意计算逻辑——单位转换、时间差计算、性能指标等。

一个 calculated item 的生命周期：

```
实现 trait → 注册到 ComputeRegistry → 通过 DashboardService 订阅 → 每帧/定时触发计算
```

## Trait 定义

### RealtimeComputeItem — 逐帧计算

逐帧触发，可持有内部状态。适用于单位转换、圈速差计算等需要按帧执行的逻辑。

```rust
pub trait RealtimeComputeItem: Send {
    /// 计算项名称。注册后通过 `calc:{name}` 格式引用。
    fn name(&self) -> &str;

    /// 对当前帧执行计算。ctx 包含当前帧数据、已计算的其他 item 结果、参考圈数据等。
    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64>;
}
```

### BatchComputeItem — 整圈批量计算

一次性处理两圈完整数据。无状态。适用于圈统计（平均速度、最快/最慢扇区等）。

```rust
pub trait BatchComputeItem: Send {
    fn name(&self) -> &str;

    fn compute_batch(
        &self,
        current_lap: &[TelemetryFrame],
        reference_lap: &[TelemetryFrame],
    ) -> ComputeResult<Vec<f64>>;
}
```

## ComputeContext 字段

```rust
pub struct ComputeContext<'a> {
    pub current_frame: &'a TelemetryFrame,     // 当前遥测帧
    pub computed_values: &'a HashMap<String, f64>,  // 已计算的前序 item 结果
    pub reference_lap: Option<&'a [TelemetryFrame]>, // 参考圈数据（可选）
    pub reference_source: Option<ReferenceSource>,   // 参考圈来源
}
```

## 注册 API

通过 `ComputeRegistry` 注册计算项：

```rust
impl ComputeRegistry {
    /// 注册 RealtimeComputeItem（逐帧计算）
    pub fn register_calc_realtime(
        &mut self,
        item: Box<dyn RealtimeComputeItem>,
    ) -> ComputeResult<()>;

    /// 注册 BatchComputeItem（整圈批量计算）
    pub fn register_calc_batch(
        &mut self,
        item: Box<dyn BatchComputeItem>,
    ) -> ComputeResult<()>;
}
```

### 校验规则

调用注册方法时，以下情况会失败：

| 校验项 | 条件 | 错误信息 |
|---|---|---|
| 空名称 | `item.name()` 返回 `""` | `注册失败: 计算项名称不能为空` |
| 重复名称 | 已存在同名 realtime 或 batch item | `注册失败: 计算项 'xxx' 已注册` |

> **注意**：realtime 和 batch **共享同一个命名空间**。如果已经注册了名为 `foo` 的 realtime item，就不能再注册名为 `foo` 的 batch item。

注册成功之后，item 即可通过 `calc:{名称}` 格式在 `DashboardService::subscribe()` 中使用。

### 注册流程总览

```
1. 定义 struct + impl RealtimeComputeItem / BatchComputeItem
       │
2. registry.register_calc_realtime(Box::new(MyItem)).unwrap();
       │                      ↑ Box::new 装箱
       │                      ↑ 返回 Result，需要 unwrap/处理
       │
3. ItemKey::parse("calc:my_item").unwrap()
       │
4. service.subscribe(key, interval, reference_source)
       │
5. service.run(frame_receiver)
```

## 示例

### 示例 1：SpeedMps — 无状态计算（单位转换）

将车速从 km/h 转换为 m/s。不需要任何外部数据，只依赖当前帧。

#### 1.1 定义计算项

```rust
use module_live_telemetry::compute::{ComputeContext, ComputeResult, items::RealtimeComputeItem};

/// 速度单位转换：公里/小时 → 米/秒
pub struct SpeedMps;

impl RealtimeComputeItem for SpeedMps {
    fn name(&self) -> &str {
        "speed_mps"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        Ok(ctx.current_frame.controls.speed_kmh as f64 / 3.6)
    }
}
```

#### 1.2 注册

```rust
use module_live_telemetry::compute::ComputeRegistry;

let mut registry = ComputeRegistry::new();

registry
    .register_calc_realtime(Box::new(SpeedMps))
    .unwrap();
```

生产代码建议处理错误：

```rust
match registry.register_calc_realtime(Box::new(SpeedMps)) {
    Ok(()) => println!("SpeedMps 注册成功"),
    Err(e) => eprintln!("注册失败: {e}"),
}
```

#### 1.3 订阅（不需要参考圈）

```rust
use module_live_telemetry::{dashboard::service::DashboardService, item_key::ItemKey};
use std::time::Duration;

service.subscribe(
    ItemKey::parse("calc:speed_mps").unwrap(),  // ← 与注册时的 name() 对应
    Duration::from_millis(50),                    // 每 50ms 计算一次
    None,                                         // 不需要参考圈
)?;
```

#### 1.4 完整端到端

```rust
use module_live_telemetry::{
    compute::{ComputeContext, ComputeResult, ComputeRegistry, items::RealtimeComputeItem},
    dashboard::{service::DashboardService, sink::ChannelSink},
    item_key::ItemKey,
};
use std::time::Duration;
use crossbeam_channel::bounded;

// 1. 定义
struct SpeedMps;
impl RealtimeComputeItem for SpeedMps {
    fn name(&self) -> &str { "speed_mps" }
    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        Ok(ctx.current_frame.controls.speed_kmh as f64 / 3.6)
    }
}

// 2. 注册
let mut registry = ComputeRegistry::new();
registry.register_calc_realtime(Box::new(SpeedMps)).unwrap();

// 3. 订阅
let (tx, rx) = bounded(10);
let sink = ChannelSink::new(tx);
let mut service = DashboardService::new(registry, Box::new(sink));
service.subscribe(
    ItemKey::parse("calc:speed_mps").unwrap(),
    Duration::from_millis(50),
    None,
).unwrap();

// 4. 运行
// service.run(frame_receiver);
```

---

### 示例 2：DeltaTimeToLifeBestLap — 有状态计算（需要参考圈）

计算当前圈与参考圈在每个赛道位置上的时间差（毫秒）。正值 = 比参考圈慢，负值 = 比参考圈快。

需要：
- 参考圈数据（从 `.acctlm2` 或旧版 `.acctlm` 文件加载）
- 内部状态（记录上次圈号、搜索索引，避免每帧从头扫描）

#### 2.1 定义计算项

```rust
use module_live_telemetry::compute::{ComputeContext, ComputeError, ComputeResult, items::RealtimeComputeItem};

pub struct DeltaTimeToLifeBestLap {
    last_lap_number: i32,
    index: usize,
}

impl DeltaTimeToLifeBestLap {
    pub fn new() -> Self {
        Self { last_lap_number: -1, index: 0 }
    }
}

impl RealtimeComputeItem for DeltaTimeToLifeBestLap {
    fn name(&self) -> &str {
        "delta_time_to_life_best_lap"
    }

    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        let reference = ctx.reference_lap
            .ok_or(ComputeError::NoValidData)?;

        if reference.is_empty() {
            return Err(ComputeError::InvalidReferenceData);
        }

        let current_lap = ctx.current_frame.session.completed_laps;
        let current_pos = ctx.current_frame.session.normalized_car_position;
        let current_time = ctx.current_frame.timing.i_current_time as f64;

        if current_lap != self.last_lap_number {
            self.last_lap_number = current_lap;
            self.index = 0;
        }

        if self.index < reference.len()
            && current_pos < reference[self.index].session.normalized_car_position
        {
            self.index = 0;
        }

        for i in self.index..reference.len() {
            let ref_time = reference[i].timing.i_current_time as f64;
            if i == reference.len() - 1
                || current_pos < reference[i + 1].session.normalized_car_position
            {
                self.index = i;
                return Ok(ref_time - current_time);
            }
        }

        Err(ComputeError::ComputationFailed(
            "无法在参考圈中找到对应位置".into(),
        ))
    }
}
```

#### 2.2 注册

```rust
let mut registry = ComputeRegistry::new();
registry
    .register_calc_realtime(Box::new(DeltaTimeToLifeBestLap::new()))
    .unwrap();
```

#### 2.3 订阅（需要参考圈数据来源）

```rust
use module_live_telemetry::compute::context::ReferenceSource;
use std::path::PathBuf;

service.subscribe(
    ItemKey::parse("calc:delta_time_to_life_best_lap").unwrap(),
    Duration::from_millis(100),
    Some(ReferenceSource {
        file_path: PathBuf::from("best_lap.acctlm2"),
        lap_number: 2,
    }),
)?;
```

`ReferenceSource` 告诉系统从 `best_lap.acctlm2` 文件中加载第 2 圈数据作为参考圈。系统会自动缓存（最多 4 圈），避免重复读文件。

#### 2.4 完整端到端

```rust
use module_live_telemetry::{
    compute::{ComputeContext, ComputeError, ComputeResult, ComputeRegistry,
              context::ReferenceSource, items::RealtimeComputeItem},
    dashboard::{service::DashboardService, sink::ChannelSink},
    item_key::ItemKey,
};
use std::path::PathBuf;
use std::time::Duration;
use crossbeam_channel::bounded;

// 1. 定义（见上文完整源码）

// 2. 注册
let mut registry = ComputeRegistry::new();
registry.register_calc_realtime(Box::new(DeltaTimeToLifeBestLap::new())).unwrap();

// 3. 订阅
let (tx, rx) = bounded(10);
let sink = ChannelSink::new(tx);
let mut service = DashboardService::new(registry, Box::new(sink));
service.subscribe(
    ItemKey::parse("calc:delta_time_to_life_best_lap").unwrap(),
    Duration::from_millis(100),
    Some(ReferenceSource {
        file_path: PathBuf::from("best_lap.acctlm2"),
        lap_number: 2,
    }),
).unwrap();

// 4. 运行
// service.run(frame_receiver);
```

---

### 示例 3：AvgSpeed — 批量计算（圈平均速度）

```rust
use module_live_telemetry::compute::{ComputeResult, items::BatchComputeItem};
use module_live_telemetry::TelemetryFrame;

pub struct AvgSpeed;

impl BatchComputeItem for AvgSpeed {
    fn name(&self) -> &str { "avg_speed" }

    fn compute_batch(
        &self,
        current_lap: &[TelemetryFrame],
        _reference_lap: &[TelemetryFrame],
    ) -> ComputeResult<Vec<f64>> {
        if current_lap.is_empty() {
            return Ok(vec![]);
        }
        let sum: f64 = current_lap.iter()
            .map(|f| f.controls.speed_kmh as f64)
            .sum();
        let avg = sum / current_lap.len() as f64;
        Ok(vec![avg])
    }
}

// 注册
let mut registry = ComputeRegistry::new();
registry.register_calc_batch(Box::new(AvgSpeed)).unwrap();
```

## 内置 Calculated Items

内置 calculated item 是编译期提供的计算项。与用户自定义的 calculated item 不同，内置项的注册逻辑由 CLI 参数驱动——用户通过 `--ref-lap` 等参数选择是否启用，而非在代码中手动注册。

### 目录 API

```rust
pub fn all_builtin_calculated_items() -> Vec<BuiltinCalcItemEntry>

pub struct BuiltinCalcItemEntry {
    /// 完整标识键，如 `calc:delta_time_to_life_best_lap`
    pub key: ItemKey,
    /// 中文描述
    pub description: &'static str,
    /// 单位，如 `Some("ms")`
    pub unit: Option<&'static str>,
    /// 是否需要参考圈数据
    pub requires_reference: bool,
}
```

使用示例：

```rust
use module_live_telemetry::compute::items::all_builtin_calculated_items;

let items = all_builtin_calculated_items();

for item in &items {
    let ref_note = if item.requires_reference { "（需要参考圈）" } else { "" };
    let unit = item.unit.unwrap_or("");
    println!("{} — {}{} [{}]", item.key, item.description, ref_note, unit);
}
// 输出:
// calc:car_x — 赛车世界坐标X [m]
// calc:car_z — 赛车世界坐标Z [m]
// calc:delta_time_to_life_best_lap — 当前圈与历史最佳圈时间差（需要参考圈）[ms]
// calc:delta_time_to_session_best_lap — 当前圈与本Session最佳圈时间差（需要参考圈）[ms]
// calc:prev_sector_time — 上一个Sector耗时 [ms]
// calc:prev_sector_number — 上一个Sector编号 []
// calc:sector_best_1 — Sector1最佳耗时 [ms]
// calc:sector_best_2 — Sector2最佳耗时 [ms]
// calc:sector_best_3 — Sector3最佳耗时 [ms]
```

### 当前内置项

| key | 描述 | 单位 | 需要参考圈 | 启用方式 |
|---|---|---|---|---|---|
| `calc:car_x` | 赛车世界坐标 X | m | 否 | 自动注册，读取 `other_cars.car_coordinates[0]` |
| `calc:car_z` | 赛车世界坐标 Z | m | 否 | 自动注册，读取 `other_cars.car_coordinates[2]` |
| `calc:delta_time_to_life_best_lap` | 当前圈与历史最佳圈时间差 | ms | 是 | `--ref-lap <文件> --ref-lap-number <N>` |
| `calc:delta_time_to_session_best_lap` | 当前圈与本Session最佳圈时间差 | ms | 是 | 运行时通过 `LapCompletedCallback` + `replace_reference()` 动态注入 |
| `calc:prev_sector_time` | 上一个 Sector 耗时 | ms | 否 | 通过 `create_prev_sector_items()` 创建并注册 |
| `calc:prev_sector_number` | 上一个 Sector 编号 | — | 否 | 通过 `create_prev_sector_items()` 创建并注册 |
| `calc:sector_best_1` | Sector 1 最佳耗时 | ms | 否 | 通过 `create_sector_best_items()` 创建并注册 |
| `calc:sector_best_2` | Sector 2 最佳耗时 | ms | 否 | 通过 `create_sector_best_items()` 创建并注册 |
| `calc:sector_best_3` | Sector 3 最佳耗时 | ms | 否 | 通过 `create_sector_best_items()` 创建并注册 |

> **扇区计算项详细逻辑**（detection、临界情况、sector index 变化处理）见 [reference/sector-calculated-items.md](../reference/sector-calculated-items.md)。

### 使用 DeltaTimeToSessionBestLap

此 item 启动时无参考圈（返回空），需要调用方通过圈完成回调动态注入：

```rust
use module_live_telemetry::{
    compute::{ComputeRegistry, context::ReferenceSource},
    recording::engine::LapCompletedEvent,
};

// 订阅（不需要 ReferenceSource）
DashboardItemSubscription::new(
    "calc:delta_time_to_session_best_lap",
    DashboardItemKind::CalculatedItem,
    Duration::from_millis(100),
);

// 圈完成回调中动态替换
let mut best_time = i32::MAX;
let session_ref = ReferenceSource {
    file_path: PathBuf::from("__session_best__"),  // 虚拟路径，仅用于缓存 key
    lap_number: 0,
};

let on_lap: LapCompletedCallback = Box::new(move |event| {
    if event.is_valid && !event.is_out_lap && event.lap_time_ms < best_time {
        best_time = event.lap_time_ms;
        registry.replace_reference(session_ref.clone(), event.lap_frames);
    }
});
```

### 在 CLI 中注册内置项

如果要在 `serve` 或 `record --dashboard` 命令中包含内置计算项，将以下代码加入对应命令函数：

```rust
// ---- serve_command / record_command 中添加 ----

// 注册 delta item
registry.register_calc_realtime(Box::new(DeltaTimeToLifeBestLap::new())).unwrap();

// 注册 sector item
let (prev_time, prev_number) = create_prev_sector_items();
registry.register_calc_realtime(Box::new(prev_time)).unwrap();
registry.register_calc_realtime(Box::new(prev_number)).unwrap();
let (sb1, sb2, sb3) = create_sector_best_items();
registry.register_calc_realtime(Box::new(sb1)).unwrap();
registry.register_calc_realtime(Box::new(sb2)).unwrap();
registry.register_calc_realtime(Box::new(sb3)).unwrap();

// 订阅
dashboard.subscribe(
    ItemKey::parse("calc:speed_mps").unwrap(),
    Duration::from_millis(interval_ms),
    None,
).unwrap();
```

## 命名空间

| 前缀 | 命名空间 | 谁管理 | 注册方式 |
|---|---|---|---|
| `raw:` | TelemetryFrame 字段 | 系统自动 | 无需注册 |
| `calc:` | 用户自定义计算项 | ComputeRegistry | `register_calc_realtime` / `register_calc_batch` |
| `system:` | 系统信息 | （未来） | （未来） |

三个命名空间**互不冲突**——`raw:speed_mps`、`calc:speed_mps`、`system:speed_mps` 可以同时存在，互不影响。

### Item 目录 API 总览

| 函数 | 返回类型 | 说明 |
|---|---|---|
| `raw_catalog::all_raw_items()` | `Vec<RawItemEntry>` | ~200 个 TelemetryFrame 字段 |
| `compute::items::all_builtin_calculated_items()` | `Vec<BuiltinCalcItemEntry>` | 内置计算项 |
| `DashboardService::list_available_items()` | `Vec<DashboardItemInfo>` | raw + calc 合并列表 |

## 错误处理

```rust
pub type ComputeResult<T> = Result<T, ComputeError>;

pub enum ComputeError {
    NoValidData,                    // 无有效数据
    InvalidReferenceData,           // 参考圈数据格式错误
    ComputationFailed(String),      // 计算过程失败
    ItemNotFound(String),           // 计算项未注册
}
```

| 阶段 | 可能错误 | 触发条件 |
|---|---|---|
| `register_calc_realtime` | `InvalidRegistration` | 空名称、重复名称 |
| `ItemKey::parse` | 返回 `None` | 格式错误（不是 `type:name`） |
| `service.subscribe` | `ItemNotFound` | calc item 未注册、raw 字段不存在 |
| `compute()` 运行时 | `NoValidData` | 参考圈缺失 |
| `compute()` 运行时 | `InvalidReferenceData` | 参考圈为空 |
| `compute()` 运行时 | `ComputationFailed` | 计算逻辑异常 |

- `compute()` 返回 `Err` 时，该帧的结果**不会写入** dashboard 输出
- 失败**不中断**其他 item 的计算
- schedule 不会推进，下一帧到达时会**立即重试**

## 关键类型速查

| 类型 | 路径 | 说明 |
|---|---|---|
| `ComputeRegistry` | `module_live_telemetry::compute` | 注册中心 |
| `DashboardService` | `module_live_telemetry::dashboard::service` | 订阅与调度 |
| `ItemKey` | `module_live_telemetry::item_key` | 统一 item 标识 |
| `ComputeContext` | `module_live_telemetry::compute::context` | 计算上下文 |
| `ReferenceSource` | `module_live_telemetry::compute::context` | 参考圈来源 |
| `RealtimeComputeRequest` | `module_live_telemetry::compute` | 实时计算请求 |
| `TelemetryFrame` | `module_live_telemetry::writer` | 遥测帧（包含所有字段） |
| `ComputeResult<T>` | `module_live_telemetry::compute` | 计算结果类型 |
| `ComputeError` | `module_live_telemetry::compute` | 错误类型 |

## 完整端到端流程

```rust
use module_live_telemetry::{
    TelemetryFrame,
    compute::{ComputeRegistry, ComputeContext, ComputeResult, items::RealtimeComputeItem},
    dashboard::{service::DashboardService, sink::ChannelSink},
    item_key::ItemKey,
};
use std::time::Duration;
use crossbeam_channel::bounded;

// 1. 实现计算项
struct MyItem;
impl RealtimeComputeItem for MyItem {
    fn name(&self) -> &str { "my_item" }
    fn compute(&mut self, ctx: &ComputeContext) -> ComputeResult<f64> {
        Ok(ctx.current_frame.controls.rpms as f64 * 1.5)
    }
}

// 2. 注册
let mut registry = ComputeRegistry::new();
registry.register_calc_realtime(Box::new(MyItem)).unwrap();

// 3. 创建 DashboardService
let (tx, rx) = bounded(10);
let sink = ChannelSink::new(tx);
let mut service = DashboardService::new(registry, Box::new(sink));

// 4. 订阅
service.subscribe(
    ItemKey::parse("calc:my_item").unwrap(),
    Duration::from_millis(50),
    None,
).unwrap();

// 5. 运行（在独立线程或主循环中）
// service.run(frame_receiver);
```
