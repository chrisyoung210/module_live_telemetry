# 阶段 4：Field ID Registry + Layout 编译器 + 协议类型

日期：2026-06-29
涉及模块：`acc-coach`（Registry + 编译器）+ `module_dashboard_protocol`（compiled 类型）
依赖：无（独立于阶段1-3，可并行启动）

---

## 1. 目标

建立字段 ID 化基础设施：
- acc-coach 侧：SQLite 持久化的 Field ID Registry + Layout 编译器（string key → id）
- module_dashboard_protocol 侧：新增 compiled 类型定义

本阶段**不改变运行时行为**——编译器输出暂不下发，仅做单元测试验证。阶段5才开始切换到 ID-based 路径。

## 2. 设计原理

### 2.1 为什么需要 ID 化

当前数据流中字段用 string key（如 `speedKmh`、`raw:controls.speed_kmh`）标识。问题：
- 每帧 `HashMap<String, f64>` 分配 string key
- IPC JSON 序列化传输 string key
- 前端 string key 查找
- remote UDP 传输 string key JSON
- alias 翻译在 120Hz 录制线程逐帧执行

Dashboard 端（local/remote/serial）**不需要知道字段语义**——它只关心"在位置 (x,y) 用字号 24 渲染一个数字"。用 numeric ID 替代 string key 可以：
- alias 翻译从每帧 120Hz 降到 layout 加载时一次
- IPC payload 从 `HashMap<String,f64>` → `Vec<(u32,f64)>`，无 string key
- remote/serial 带宽显著降低
- 前端 number key 查找更快

### 2.2 关键设计决策

| 决策 | 选择 | 理由 |
|---|---|---|
| ID 稳定性 | SQLite 持久化，跨会话稳定 | remote 设备端缓存定义更友好，layout 增量更新 |
| 注册名 | user-facing name（如 `speedKmh`） | alias 翻译与 ID 注册合并为一步 |
| layout 存储格式 | 保持 string key 不变 | 用户可读、可编辑、可跨设备共享 |
| layout 下发格式 | compiled id-based | 运行时高效 |
| 兼容性 | 硬切换（阶段5） | remote 设备端未开发，无遗留兼容 |
| 模板编译范围 | 只编译 `{...}` 占位符 | 裸 identifier 在表达式中会抛错，不存在有效场景 |
| Designer 预览 | 独立路径，不走编译 | 用户编辑时看 string key，预览用 string-key frame |

## 3. 改动方案

### 3.1 D1：acc-coach Field ID Registry

**新增文件**：`src/dashboard/field_registry.rs`

#### 3.1.1 SQLite 表

新增 migration（当前最高 v11，新增 v12）：

**新增文件**：`src/db/migrations_v12.sql`

```sql
CREATE TABLE IF NOT EXISTS dashboard_field_registry (
    field_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    field_name  TEXT    NOT NULL UNIQUE,
    kind        TEXT    NOT NULL DEFAULT 'raw',
    registered_at TEXT  NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_dashboard_field_registry_name
    ON dashboard_field_registry(field_name);
```

**改动文件**：`src/db/migrations.rs` — 增加 v12 migration 调用

#### 3.1.2 Registry 结构

```rust
// src/dashboard/field_registry.rs

use std::collections::HashMap;
use std::sync::Mutex;
use rusqlite::Connection;

pub type FieldId = u32;

#[derive(Debug, Clone)]
pub struct FieldDefinition {
    pub id: FieldId,
    pub name: String,
    pub kind: String,  // "raw" | "calc" | "system"
}

pub struct FieldRegistry {
    by_name: HashMap<String, FieldId>,
    by_id: HashMap<FieldId, FieldDefinition>,
    next_id: FieldId,
}

impl FieldRegistry {
    /// 从数据库加载已有注册项
    pub fn load_from_db(conn: &Connection) -> Self {
        let mut by_name = HashMap::new();
        let mut by_id = HashMap::new();
        let mut max_id: u32 = 0;

        let mut stmt = conn.prepare(
            "SELECT field_id, field_name, kind FROM dashboard_field_registry"
        ).unwrap();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }).unwrap();
        for row in rows {
            let (id, name, kind) = row.unwrap();
            by_name.insert(name.clone(), id);
            by_id.insert(id, FieldDefinition { id, name, kind });
            if id > max_id { max_id = id; }
        }

        Self {
            by_name,
            by_id,
            next_id: max_id + 1,
        }
    }

    /// 注册字段（已存在则返回已有 ID，不存在则分配新 ID 并持久化）
    pub fn register(&mut self, name: &str, conn: &Connection) -> FieldId {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        // 持久化
        conn.execute(
            "INSERT INTO dashboard_field_registry (field_id, field_name, kind) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, name, "raw"],
        ).ok();
        self.by_name.insert(name.to_string(), id);
        self.by_id.insert(id, FieldDefinition {
            id,
            name: name.to_string(),
            kind: "raw".to_string(),
        });
        id
    }

    /// 查询 ID（已注册）
    pub fn id_for(&self, name: &str) -> Option<FieldId> {
        self.by_name.get(name).copied()
    }

    /// 查询名称（已注册）
    pub fn name_for(&self, id: FieldId) -> Option<&str> {
        self.by_id.get(&id).map(|d| d.name.as_str())
    }

    /// 所有已注册定义
    pub fn definitions(&self) -> Vec<FieldDefinition> {
        self.by_id.values().cloned().collect()
    }
}
```

#### 3.1.3 注册名用 user-facing name

Registry 注册时使用 **user-facing name**（如 `speedKmh`），而非 canonical name（如 `raw:controls.speed_kmh`）。

这意味着 alias 翻译与 ID 注册合并为一步：
- `module_live_telemetry` 输出 canonical key（`raw:controls.speed_kmh`）
- acc-coach 用 `ChannelAliasTable::to_user_facing` 翻译为 `speedKmh`
- 用 `speedKmh` 注册到 Registry，得到 ID（如 42）
- 后续帧中 `speedKmh` → ID 42 的映射稳定

### 3.2 D2：acc-coach Layout 编译器

**新增文件**：`src/dashboard/compiler.rs`

#### 3.2.1 编译逻辑

```rust
// src/dashboard/compiler.rs

use module_dashboard_protocol::{
    DashboardControl, DashboardLayoutPayload, RegisteredDashboardLayout,
    ChartFieldConfig, DashboardConditionalRule,
};
use crate::dashboard::field_registry::{FieldRegistry, FieldId};
use rusqlite::Connection;

/// 编译后的控件定义（id-based）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledDashboardControl {
    pub id: String,
    pub widget_type: String,
    pub telemetry_field_id: Option<FieldId>,
    pub compiled_text_template: String,
    pub format: Option<String>,
    pub refresh_hz: f64,
    pub x: f64, pub y: f64, pub width: f64, pub height: f64,
    pub visible: bool,
    pub visible_expression: Option<String>,
    pub font_family: String,
    pub font_size: f64,
    pub text_color: String,
    pub text_color_expression: Option<String>,
    pub text_fallback: Option<String>,
    pub background_color: String,
    pub background_color_expression: Option<String>,
    pub background_image_data_url: Option<String>,
    pub horizontal_center: bool,
    pub vertical_center: bool,
    pub chart_fields: Vec<CompiledChartFieldConfig>,
    pub chart_sample_count: Option<u32>,
    pub track_id: Option<String>,
    pub dot_color: Option<String>,
    pub dot_size: Option<f64>,
    pub conditional_rules: Vec<CompiledConditionalRule>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledChartFieldConfig {
    pub field_id: FieldId,
    pub color: String,
    pub label: String,
    pub default_value: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledConditionalRule {
    pub target: String,
    pub field_id: FieldId,
    pub operator: String,
    pub compare_value: f64,
    pub color: String,
}

/// 编译后的布局
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledDashboardLayout {
    pub layout_id: String,
    pub canvas_width: f64,
    pub canvas_height: f64,
    pub image_mime: String,
    pub static_image_base64: String,
    pub controls: Vec<CompiledDashboardControl>,
}

/// 编译单个 layout
pub fn compile_layout(
    registered: &RegisteredDashboardLayout,
    registry: &mut FieldRegistry,
    conn: &Connection,
    alias: &crate::dashboard::alias::ChannelAliasTable,
) -> CompiledDashboardLayout {
    let payload = &registered.payload;
    let controls = payload.controls.iter()
        .map(|control| compile_control(control, registry, conn, alias))
        .collect();

    CompiledDashboardLayout {
        layout_id: registered.id.clone(),
        canvas_width: payload.canvas_width,
        canvas_height: payload.canvas_height,
        image_mime: payload.image_mime.clone(),
        static_image_base64: payload.static_image_base64.clone(),
        controls,
    }
}

fn compile_control(
    control: &DashboardControl,
    registry: &mut FieldRegistry,
    conn: &Connection,
    alias: &crate::dashboard::alias::ChannelAliasTable,
) -> CompiledDashboardControl {
    // 编译 telemetry_field
    let telemetry_field_id = control.telemetry_field.as_ref()
        .map(|field| resolve_field_id(field, registry, conn, alias));

    // 编译 text_template 中的 {fieldName} → {id}
    let compiled_text_template = compile_text_template(
        &control.text_template, registry, conn, alias,
    );

    // 编译 visible_expression / text_color_expression / background_color_expression
    let visible_expression = control.visible_expression.as_ref()
        .map(|expr| compile_text_template(expr, registry, conn, alias));
    let text_color_expression = control.text_color_expression.as_ref()
        .map(|expr| compile_text_template(expr, registry, conn, alias));
    let background_color_expression = control.background_color_expression.as_ref()
        .map(|expr| compile_text_template(expr, registry, conn, alias));

    // 编译 chart_fields
    let chart_fields = control.chart_fields.iter()
        .map(|cf| compile_chart_field(cf, registry, conn, alias))
        .collect();

    // 编译 conditional_rules
    let conditional_rules = control.conditional_rules.iter()
        .map(|rule| compile_conditional_rule(rule, registry, conn, alias))
        .collect();

    CompiledDashboardControl {
        id: control.id.clone(),
        widget_type: format!("{:?}", control.widget_type).to_lowercase(),
        telemetry_field_id,
        compiled_text_template,
        format: control.format.clone(),
        refresh_hz: control.refresh_hz,
        x: control.x, y: control.y, width: control.width, height: control.height,
        visible: control.visible,
        visible_expression,
        font_family: control.font_family.clone(),
        font_size: control.font_size,
        text_color: control.text_color.clone(),
        text_color_expression,
        text_fallback: control.text_fallback.clone(),
        background_color: control.background_color.clone(),
        background_color_expression,
        background_image_data_url: control.background_image_data_url.clone(),
        horizontal_center: control.horizontal_center,
        vertical_center: control.vertical_center,
        chart_fields,
        chart_sample_count: control.chart_sample_count,
        track_id: control.track_id.clone(),
        dot_color: control.dot_color.clone(),
        dot_size: control.dot_size,
        conditional_rules,
    }
}

/// 解析字段名为 ID
/// 接受 user-facing name（如 "speedKmh"）或 canonical name（如 "raw:controls.speed_kmh"）
fn resolve_field_id(
    field: &str,
    registry: &mut FieldRegistry,
    conn: &Connection,
    alias: &crate::dashboard::alias::ChannelAliasTable,
) -> FieldId {
    // 如果已经是 user-facing name，直接注册
    if alias.is_user_format(field) {
        return registry.register(field, conn);
    }
    // 如果是 canonical name，翻译为 user-facing 再注册
    if let Some(user_name) = alias.to_user_facing(field) {
        return registry.register(user_name, conn);
    }
    // 无法翻译，用原名注册（fallback）
    registry.register(field, conn)
}

/// 编译文本模板：{fieldName} → {id}
/// 只处理 {...} 占位符，不处理裸 identifier
fn compile_text_template(
    template: &str,
    registry: &mut FieldRegistry,
    conn: &Connection,
    alias: &crate::dashboard::alias::ChannelAliasTable,
) -> String {
    // 正则匹配 {fieldName} 或 {fieldName|format}
    // 将 fieldName 替换为 ID
    // {value} 保持不变（运行时绑定到 control.telemetry_field_id）
    // {{expr:...}} 内部的 {fieldName} 也要替换
    template.replace(/\{([^{}]+)\}/g, |match: &str| {
        let token = &match[1..match.len()-1];  // 去掉 { }
        let parts: Vec<&str> = token.split('|').collect();
        let field = parts[0].trim();
        if field == "value" {
            return match.to_string();  // {value} 保持不变
        }
        let id = resolve_field_id(field, registry, conn, alias);
        if parts.len() > 1 {
            return format!("{{{id}|{}}}", parts[1..].join("|"));
        }
        format!("{{{id}}}")
    })
}
```

**注意**：上面是伪代码（Rust 没有 `replace` 闭包形式），实际实现需要用 `regex` crate 或手动解析。当前 acc-coach 已有 `regex` 依赖吗？需要检查 Cargo.toml。如果没有，手动解析 `{...}` 也不难。

#### 3.2.2 编译时序

编译发生在"layout 加载用于下发"时：
- overlay 窗口请求 layout → acc-coach 编译后返回 `CompiledDashboardLayout`
- remote 设备连接 → acc-coach 编译后通过 TCP 下发
- **DashboardDesignerView 预览不走编译**（用户编辑时看 string key）

#### 3.2.3 本阶段不接入运行时

编译器实现后，仅做单元测试验证编译正确性。不改变现有 IPC 返回类型、FrameBus 类型。阶段5才切换到编译后的下发路径。

### 3.3 D4：module_dashboard_protocol 新增 compiled 类型

**改动文件**：`module_dashboard_protocol/src/lib.rs`、`module_dashboard_protocol/types/index.ts`

新增 compiled 类型（与现有类型并行，不修改现有 `DashboardControl` 等）：

```rust
// module_dashboard_protocol/src/lib.rs 新增

pub type DashboardFieldId = u32;

/// ID-based 数据帧（替代 DashboardValuesFrame 的 HashMap<String,f64>）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardValuesFrameV2 {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub values: Vec<(DashboardFieldId, f64)>,
}

/// 编译后的 chart field 配置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledChartFieldConfig {
    pub field_id: DashboardFieldId,
    pub color: String,
    pub label: String,
    #[serde(default)]
    pub default_value: Option<f64>,
}

/// 编译后的条件规则
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledConditionalRule {
    pub target: String,
    pub field_id: DashboardFieldId,
    pub operator: String,
    pub compare_value: f64,
    pub color: String,
}

/// 编译后的控件（id-based，下发格式）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledDashboardControl {
    pub id: String,
    pub widget_type: WidgetType,
    pub telemetry_field_id: Option<DashboardFieldId>,
    pub compiled_text_template: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default = "default_control_refresh_hz")]
    pub refresh_hz: f64,
    // 布局属性
    pub x: f64, pub y: f64, pub width: f64, pub height: f64,
    // 可见性
    #[serde(default = "default_control_visible")]
    pub visible: bool,
    #[serde(default)]
    pub visible_expression: Option<String>,
    // 外观
    #[serde(default)]
    pub font_family: String,
    #[serde(default)]
    pub font_size: f64,
    #[serde(default)]
    pub text_color: String,
    #[serde(default)]
    pub text_color_expression: Option<String>,
    #[serde(default)]
    pub text_fallback: Option<String>,
    #[serde(default)]
    pub background_color: String,
    #[serde(default)]
    pub background_color_expression: Option<String>,
    #[serde(default)]
    pub background_image_data_url: Option<String>,
    #[serde(default)]
    pub horizontal_center: bool,
    #[serde(default)]
    pub vertical_center: bool,
    // Chart
    #[serde(default)]
    pub chart_fields: Vec<CompiledChartFieldConfig>,
    #[serde(default)]
    pub chart_sample_count: Option<u32>,
    // Map
    #[serde(default)]
    pub track_id: Option<String>,
    #[serde(default)]
    pub dot_color: Option<String>,
    #[serde(default)]
    pub dot_size: Option<f64>,
    // 条件规则
    #[serde(default)]
    pub conditional_rules: Vec<CompiledConditionalRule>,
}

/// 编译后的布局（id-based，下发格式）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledDashboardLayout {
    pub layout_id: String,
    pub canvas_width: f64,
    pub canvas_height: f64,
    pub image_mime: String,
    pub static_image_base64: String,
    pub controls: Vec<CompiledDashboardControl>,
}
```

TypeScript 镜像类型同步新增到 `module_dashboard_protocol/types/index.ts`。

**注意**：这是给独立会话的 PRD 文档。acc-coach 侧的 `compiler.rs` 中的 `CompiledDashboardControl` 应直接复用 `module_dashboard_protocol` 中的类型，而非自定义。

## 4. 模块间开发顺序

| 模块 | 改动 | 并行可行性 |
|---|---|---|
| module_dashboard_protocol | 新增 compiled 类型（D4） | 独立，不修改现有类型 |
| acc-coach | Field Registry + 编译器（D1+D2） | 依赖 D4 的类型定义 |

**开发顺序**：

1. **module_dashboard_protocol 先出类型**（D4）— 独立，不影响任何现有功能
2. **acc-coach 实现 Registry + 编译器**（D1→D2）— D2 依赖 D1，且 D2 引用 D4 的类型

两者可部分并行：D1（Registry）不依赖 D4，可先做；D2（编译器）依赖 D4 类型就绪。

## 5. 验收标准

| 验收项 | 验证方法 |
|---|---|
| SQLite migration v12 成功 | `Database::open` + `migrate` 后表存在 |
| Registry 加载/注册/查询 | 单元测试：注册字段 → 查询 ID → 查询 name → 重启后 ID 稳定 |
| 编译器输出正确 | 单元测试：给定 layout（string key）→ 编译为 compiled layout（id）→ 验证 ID 正确 |
| 模板编译正确 | 单元测试：`"Speed: {speedKmh} km/h"` → `"Speed: {42} km/h"`；`{value}` 保持不变 |
| 表达式模板编译 | 单元测试：`"{{expr:round({speedKmh}, 2)}}"` → `"{{expr:round({42}, 2)}}"` |
| 条件规则编译 | 单元测试：`rule.telemetry_field: "speedKmh"` → `rule.field_id: 42` |
| **运行时无变化** | 启动 ACC + recording，overlay 行为与改动前完全一致 |

## 6. 风险

| 风险 | 缓解 |
|---|---|
| Registry 并发访问 | Registry 在 acc-coach 主线程使用，`auto_recording_loop` 通过 command 交互；如需跨线程，用 `Arc<Mutex<FieldRegistry>>` |
| SQLite 写入性能 | 注册只在 layout 加载时发生（低频），不影响热路径 |
| 编译器正则解析 | 手动解析 `{...}` 也很简单；注意处理嵌套 `{{expr:...}}` |
| 新增类型与现有类型混淆 | compiled 类型命名清晰（`Compiled` 前缀），不修改现有类型 |

## 7. 参照文档

- `docs/acc-coach/dashboard-architecture.md` — 模块边界
- `docs/acc-coach/prd/module-dashboard-protocol-type-alignment.md` — 现有协议类型对齐 PRD
- README.md 关键代码位置索引 — `DashboardFieldRegistry` / `DashboardCompactPatch` 条目（module_live_telemetry 已有类似实现可参考）
