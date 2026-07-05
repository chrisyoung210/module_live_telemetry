# Stage 2：acc-coach — 编译期解析与订阅

> **前置依赖**：Stage 1（`CompiledFieldBinding` 类型、role 常量）
> **本 Stage 产物**：`widget_bindings.rs`、compiler 写入 bindings、writer 按 bindings 订阅

## 目标

集中管理 widget 内置 binding 关系，作为编译期 fieldId 解析和运行期 telemetry 订阅的**唯一数据源**，消除当前 `writer.rs` 硬编码 map 订阅与 compiler 之间的割裂。

## 改动清单

### 2.1 新增 `src/dashboard/widget_bindings.rs`

```rust
use crate::dashboard::layout::WidgetType; // 沿用 writer.rs:11 的导入路径
use module_dashboard_protocol::{ROLE_CAR_X, ROLE_CAR_Z};

pub struct WidgetBinding {
    pub role: &'static str,
    pub field: &'static str,
}

pub fn builtin_widget_bindings(widget_type: WidgetType) -> &'static [WidgetBinding] {
    match widget_type {
        WidgetType::Map => &[
            WidgetBinding { role: ROLE_CAR_X, field: "calc:car_x" },
            WidgetBinding { role: ROLE_CAR_Z, field: "calc:car_z" },
        ],
        _ => &[],
    }
}
```

**alias 行为注释（重要，勿误改）**：`field` 使用 canonical 形式 `"calc:car_x"` 是有意为之。
- `resolve_field_id("calc:car_x")` 经 alias 层（`alias.rs` 将 `calc:car_x` → user-facing `"carX"`）最终注册为 `"carX"`。
- `recording_dashboard_item_for_field("calc:car_x")` 产出 item_name `"calc:car_x"`。
- 两者名称不同但经 alias bridge 对齐到同一 fieldId，与 text widget 用 `"speedKmh"` 走同一条路径，已验证可行。
- 勿将 `field` 误改为 `"carX"`（行为等价但偏离 writer.rs 既有 canonical 约定）。

### 2.2 `src/dashboard/compiler.rs`

在 `compile_control` 中：
- 调用 `resolve_field_id` 把 `builtin_widget_bindings(control.widget_type)` 解析成数字 ID。
- 将解析结果写入 `CompiledDashboardControl.bindings`：

```rust
let bindings = builtin_widget_bindings(control.widget_type)
    .iter()
    .map(|b| CompiledFieldBinding {
        role: b.role.to_string(),
        field_id: resolve_field_id(b.field, registry, conn, alias),
    })
    .collect();

// 在 CompiledDashboardControl 构造中：
// bindings,
```

### 2.3 `src/recording/writer.rs`

将写死的 map widget 订阅逻辑：

```rust
if control.widget_type == WidgetType::Map {
    for car_field in ["calc:car_x", "calc:car_z"] { ... }
}
```

替换为读取同一数据源：

```rust
for binding in builtin_widget_bindings(control.widget_type) {
    if let Some((item_name, item_kind)) = recording_dashboard_item_for_field(binding.field, alias) {
        if seen.insert(item_name.clone()) {
            subscriptions.push(DashboardItemSubscription::new(item_name, item_kind, interval));
        }
    }
}
```

**职责说明**：bindings 的订阅不走 `telemetry_fields_for_control`（该函数只管 `telemetry_field` / `text_template` / `conditional_rules` / `chart_fields`），是与之并行的第二条来源。`dashboard_subscriptions_for_local_layouts` 内有 `seen` BTreeSet 去重，两者不会重复订阅。

## 验证

- [ ] `list_registered_dashboard_layouts` 返回的 map control JSON 包含 `bindings: [{role: "carX", fieldId: N}, {role: "carZ", fieldId: M}]`。
- [ ] `poll_dashboard_frame` / `dashboard://frame` 事件中的 V2 frame 包含这两个 fieldId 的数值。
- [ ] 非 Map widget 的 `bindings` 为空 `[]`，不影响现有 text / chart 编译。

## 单测补充

- `compile_control_map_bindings`：验证 Map widget 编译后 `bindings` 含 `[{role:"carX", field_id:N}, {role:"carZ", field_id:M}]`，且 ID 与 `FieldRegistry` 一致；`Text`/`Chart` widget 编译后 `bindings` 为空。
- writer 单测：`dashboard_subscriptions_for_local_layouts` 对含 Map widget 的 layout 产出 `calc:car_x` + `calc:car_z` 订阅（经 `builtin_widget_bindings` 路径）。
