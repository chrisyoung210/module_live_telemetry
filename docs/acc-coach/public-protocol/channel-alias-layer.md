# 通道别名中间层设计

> 所属模块：`acc-coach`
>
> 接口边界：`acc-coach` ↔ `module_live_telemetry`
>
> 使用者：`src/recording/writer.rs`、`src/dashboard/output.rs`、`src/dashboard/mod.rs`
>
> 状态：已定稿

---

## 一、设计目标

在当前架构中，通道别名逻辑散布在 7+ 个文件中，导致每个模块都同时感知"规范格式"(`raw:controls.speed_kmh`) 和"用户格式"(`speedKmh`)。这违背了别名层作为反损坏层的设计初衷。

本次设计建立一个**唯一的别名中间层** `src/dashboard/alias.rs`，负责所有规范格式与用户格式之间的转换。别名层之外，`acc-coach` 所有代码只感知用户格式。

---

## 二、接口定义

```rust
/// 通道别名表 — 规范格式与用户格式之间的唯一转换点。
///
/// 从 `module_live_telemetry::raw_catalog::all_raw_items()` 构建，
/// 结合规则自动生成 + 显式覆盖表。
pub struct ChannelAliasTable {
    /// 用户名 → 规范名（含同义词 key）
    user_to_canonical: HashMap<String, String>,
    /// 规范名 → 主用户名（不含同义词）
    canonical_to_user: HashMap<String, String>,
}

impl ChannelAliasTable {
    /// 从 raw_catalog 构建，初始化时调用一次。
    ///
    /// 构建流程：
    ///   1. 遍历 all_raw_items()
    ///   2. 对每个 entry.key.name ("controls.speed_kmh")
    ///       a. 去掉子系统前缀 → "speed_kmh"
    ///       b. snake_case → camelCase → "speedKmh"
    ///       c. 查覆盖表，命中则替换
    ///   3. 登记同义词映射
    pub fn from_catalog() -> Self;

    /// 用户名 → 规范名（构建订阅时使用）。
    ///
    /// 例: to_canonical("speedKmh") → Some("raw:controls.speed_kmh")
    ///     同时接受同义词: to_canonical("sessionBestLapTimeMs") → Some("raw:timing.i_best_time")
    pub fn to_canonical(&self, user_name: &str) -> Option<&str>;

    /// 规范名 → 主用户名（发布 frame 时使用）。
    ///
    /// 例: to_user_facing("raw:controls.speed_kmh") → Some("speedKmh")
    ///     只返回主名称，如 "bestLapTimeMs"，不返回同义词 "sessionBestLapTimeMs"
    pub fn to_user_facing(&self, canonical: &str) -> Option<&str>;

    /// 判断一个字符串是否已经是用户格式（不含 raw:/calc:/system: 前缀）。
    /// 用于 subscription 构建时的快速路径判断。
    pub fn is_user_format(&self, name: &str) -> bool;
}
```

### 构建逻辑伪代码

```rust
fn from_catalog() -> Self {
    let mut user_to_canonical = HashMap::new();
    let mut canonical_to_user = HashMap::new();

    // 覆盖表
    let overrides: HashMap<&str, &str> = [
        ("controls.gas", "throttlePct"),
        ("controls.brake", "brakePct"),
        ("controls.clutch", "clutchPct"),
        ("controls.rpms", "rpm"),
        ("controls.steer_angle", "steerRawAngle"),
        ("timing.i_current_time", "currentLapTimeMs"),
        ("timing.i_last_time", "lastLapTimeMs"),
        ("timing.i_best_time", "bestLapTimeMs"),
        ("timing.i_delta_lap_time", "bestLapDeltaTimeMs"),
        ("timing.i_estimated_lap_time", "predictedLapTimeByBest"),
    ].into();

    // 同义词表（主名 → 规范名已被覆盖表处理）
    let synonyms: &[(&str, &[&str])] = &[
        ("bestLapTimeMs", &["sessionBestLapTimeMs"]),
        ("bestLapDeltaTimeMs", &["sessionLapDeltaTimeMs"]),
        ("predictedLapTimeByBest", &["predictedLapTimeBySession"]),
        ("steerRawAngle", &["steeringDeg"]),
    ];

    for entry in raw_catalog::all_raw_items() {
        let canonical = entry.key.to_string();       // "raw:controls.speed_kmh"
        let name = entry.key.name();                  // "controls.speed_kmh"

        // 去前缀 + camelCase
        let user_name = if let Some(override_name) = overrides.get(name) {
            override_name.to_string()
        } else if let Some((_prefix, field)) = name.split_once('.') {
            snake_to_camel(field)
        } else {
            snake_to_camel(name)
        };

        canonical_to_user.insert(canonical.clone(), user_name.clone());
        user_to_canonical.insert(user_name.clone(), canonical.clone());
    }

    // 注册同义词
    for (primary, syns) in synonyms {
        for syn in *syns {
            if let Some(canonical) = user_to_canonical.get(*primary) {
                user_to_canonical.insert(syn.to_string(), canonical.clone());
            }
        }
    }

    Self { user_to_canonical, canonical_to_user }
}
```

---

## 三、调用方改造

### 3.1 `src/recording/writer.rs` — 订阅构建

**`recording_dashboard_item_for_field()` 改造**：

```rust
// 当前：硬编码 20 行 match
"speedKmh" => "raw:controls.speed_kmh",
"throttlePct" => "raw:controls.gas",
// ...

// 目标：委托别名层
pub(crate) fn recording_dashboard_item_for_field(
    channel: &str,
    alias: &ChannelAliasTable,
) -> Option<(String, DashboardItemKind)> {
    let channel = channel.trim();
    if channel.is_empty() { return None; }

    // 已经是规范格式，直接使用
    if let Some((prefix, _)) = channel.split_once(':') {
        let kind = match prefix {
            "raw" => DashboardItemKind::RawItem,
            "calc" => DashboardItemKind::CalculatedItem,
            "system" => DashboardItemKind::SystemItem,
            _ => return None,
        };
        return Some((channel.to_string(), kind));
    }

    // 用户格式 → 规范格式（通过别名层）
    let canonical = alias.to_canonical(channel)?;
    Some((canonical.to_string(), DashboardItemKind::RawItem))
}
```

### 3.2 `src/recording/writer.rs` — Frame 发布

**`recording_dashboard_fields()` 改造**：

```rust
// 当前：双重 emit + 硬编码反向别名
insert_value(&mut fields, "speedKmh", value); // 用户 key
// 同时也保留了 raw key...

// 目标：只 emit 用户 key，通过别名层翻译
pub fn recording_dashboard_fields(
    data: HashMap<String, f64>,
    alias: &ChannelAliasTable,
) -> BTreeMap<String, serde_json::Value> {
    let mut fields = BTreeMap::new();
    insert_value(&mut fields, "timestampMs", Utc::now().timestamp_millis() as f64);

    for (name, value) in data {
        // 规范 key → 用户 key
        if let Some(user_key) = alias.to_user_facing(&name) {
            insert_value(&mut fields, user_key, value);
        }
        // 保留原始值以兼容未在别名表中的通道
        if !alias.is_user_format(&name) {
            insert_value(&mut fields, &name, value);
        }
    }
    fields
}
```

> **有意例外说明**：上述 `if !alias.is_user_format(&name)` 分支会保留未登记通道的原始（规范格式）key，使得 `frame.values` 在过渡期仍可能含规范格式 key。这是有意兜底，用于 `raw_catalog` 尚未覆盖全部通道时的渐进迁移，与 `channel-naming-convention.md` 的"铁律：别名层之外任何代码不得同时感知两种格式"存在表述张力。**此兜底仅存在于别名层内部**，下游消费方仍只感知用户格式；待 `raw_catalog` 覆盖完整后该分支应移除。

### 3.3 `src/dashboard/output.rs` — 实时 Frame

**`dashboard_fields_map()` 改造**：当前手工构造用户 key（如 `insert_value(&mut fields, "speedKmh", frame.speed_kmh)`），改为通过别名表统一查找：

```rust
pub fn dashboard_fields_map(
    frame: &LiveFrame,
    alias: &ChannelAliasTable,
) -> BTreeMap<String, serde_json::Value> {
    // ... 构建临时映射: LiveFrame 字段 → 用户 key
    // 通过 alias.to_user_facing() 统一产出
}
```

> 注意：`LiveFrame` 没有规范 key，它的字段名是 Rust struct 的 snake_case 字段名（如 `speed_kmh`）。需要额外维护一个 `LiveFrame 字段名 → 规范 key` 的映射，或者直接在这层使用别名表查找。

### 3.4 `src/dashboard/mod.rs` — Channel ID 统一

**`raw_channel_definition()` 改造**：

```rust
// 当前：id = entry.key.to_string() → "raw:controls.speed_kmh"
// 目标：id = alias.to_user_facing(&entry.key.to_string()).unwrap_or(entry.key.to_string())
```

> **连带影响（必须同步处理）**：`raw_channel_definition()` 内部用此 `id` 查 `capture.fields.get(&id)`（`src/dashboard/mod.rs:591`）取采样率。`capture.fields` 的 key 当前是规范格式（`raw:controls.speed_kmh`），若把 `id` 改为用户格式直接查将查不到配置。改造时需在函数内部保留一份规范格式 key 专门用于查 `capture.fields`：

```rust
fn raw_channel_definition(
    entry: &RawItemEntry,
    capture: &LiveCaptureConfig,
    alias: &ChannelAliasTable,
) -> ChannelDefinition {
    let canonical = entry.key.to_string();                    // 规范格式，用于查 capture.fields
    let id = alias.to_user_facing(&canonical)                 // 用户格式，对外暴露
        .map(str::to_string)
        .unwrap_or_else(|| canonical.clone());
    let default_hz = capture
        .fields
        .get(&canonical)                                      // 用规范格式查，保持兼容
        .map(|field| field.sample_hz)
        .unwrap_or(DEFAULT_RAW_SAMPLE_HZ);
    // ... 其余字段同现有实现，label/group 等可继续基于 entry
}
```

同时需检查 `build_channel_registry`（`src/dashboard/mod.rs:414`）下游是否还有其他地方用 channel id 反查 `capture.fields`，统一改为规范格式查询。

### 3.5 `src/ipc/mod.rs` — 日志去双感知

`log_live_dashboard_ipc_frame()` 删除 `.or_else()` 回退查规范 key。

### 3.6 `src/recording/auto.rs` — 调试日志去多 key 回退

删除 `dashboard_value()` 辅助函数，单 key 查找。

---

## 四、初始化与生命周期

`ChannelAliasTable` 在 `src/main.rs` 启动时构建一次，注入到需要使用的模块：

```rust
// main.rs
let alias_table = Arc::new(ChannelAliasTable::from_catalog());
app.manage(alias_table);
```

后续 `recording/writer`、`dashboard/output` 等通过 Tauri State 获取。

---

## 五、与 `module_live_telemetry` 的接口契约

本别名层位于 `acc-coach` 侧，不要求 `module_live_telemetry` 做任何改动。契约如下：

| 方向 | 格式 | 接口 |
|------|------|------|
| acc-coach → module_live_telemetry | 规范格式 | `DashboardItemSubscription.item_name` = `"raw:controls.speed_kmh"` |
| module_live_telemetry → acc-coach | 规范格式 | `DashboardValuesFrame.values` 中 key 为规范格式（被别名层翻译后仅暴露用户格式给下游） |

---

## 六、与 `module_local_dashboard` 的接口契约

| 方向 | 格式 | 接口 |
|------|------|------|
| acc-coach → module_local_dashboard | 用户格式 | `DashboardValuesFrame.values` key = `"speedKmh"` |
| acc-coach → module_local_dashboard | 用户格式 | `layout.controls[].telemetryField` = `"speedKmh"` |

---

## 七、修订记录

| 日期 | 修订内容 |
|------|----------|
| 2026-06-27 | 初版，定义接口 + 构建逻辑 + 调用方改造概要 |
