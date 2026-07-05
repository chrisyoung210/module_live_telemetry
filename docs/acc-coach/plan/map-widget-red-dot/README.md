# Map Widget 红点修复方案：通用 Widget Field Bindings

## 背景与问题

修改实时 dashboard 数据传输逻辑后，local dashboard overlay 中的 Map Widget 不再显示代表赛车位置的红点。

根因：Map Widget 渲染需要两个 live telemetry 字段（`car_x`、`car_z`），但编译后的布局 `CompiledDashboardControl` 没有携带这两个字段的数字 ID。Overlay 收到 V2 数据帧后，无法知道该用哪两个 `fieldId` 来画红点。

次生根因：`dashboardRenderer.tsx` 中 `MapWidget` 通过 `control as CompiledDashboardControl & MapWidgetFieldIds` 类型断言读取 `carXFieldId`/`carZFieldId`，但 `MapWidgetFieldIds` 是从未被填充的占位接口（`types.ts` 注释自认 "pending addition"），两个字段恒为 `undefined`，红点永远不画。

## 设计目标

1. **功能隔离**
   - `module_dashboard_protocol` 只提供通用协议结构，不感知具体 widget 类型。
   - `acc-coach` 负责维护 widget 内部语义与 telemetry channel 的映射（因为它拥有 `FieldRegistry` 和别名层）。
   - `module_local_dashboard` 只负责"拿到 field ID 后如何渲染"，不直接依赖具体 channel 名。

2. **可扩展**
   - 任何 widget 未来需要多个字段绑定时，不需要再改协议，只需新增 binding role。

3. **Role 契约可靠**
   - `role` 是 acc-coach（赋值方）与 `module_local_dashboard`（解释方）之间的字符串契约。为避免拼写错误静默失效，role 名以常量形式在 `module_dashboard_protocol` 导出，双方引用常量而非字面量。protocol 层只导出字符串常量，不引用 `WidgetType`，不破坏"不感知 widget 类型"原则。

## 核心设计

在 `CompiledDashboardControl` 中新增通用 `bindings` 字段：

```rust
#[serde(rename_all = "camelCase")]
pub struct CompiledFieldBinding {
    pub role: String,               // widget 内部语义，例如 "carX"
    pub field_id: DashboardFieldId, // FieldRegistry 数字 ID
}

pub struct CompiledDashboardControl {
    // ... 原有字段 ...
    #[serde(default)]
    pub bindings: Vec<CompiledFieldBinding>,
}
```

- `role`：由 `module_local_dashboard` 定义和解释。protocol 层导出共享常量（如 `pub const ROLE_CAR_X: &str = "carX";`），双方引用常量。
- `field_id`：由 `acc-coach` 编译时从 `FieldRegistry` 分配。
- `CompiledFieldBinding` 必须自带 `#[serde(rename_all = "camelCase")]`：`CompiledDashboardControl` 的 camelCase rename 不会自动传递给嵌套 struct，缺失会导致 TS 端拿到 `field_id` 而非 `fieldId`。
- TS 端类型为 `bindings: CompiledFieldBinding[]`（非可选）。Rust 侧 `#[serde(default)]` 保证 JSON 中 `bindings` 始终出现且为 `[]`，TS 端不需要可选链。

## Stage 索引

| Stage | 模块 | 文档 | 前置 |
|-------|------|------|------|
| 1 | `module_dashboard_protocol` | [stage-1-module-dashboard-protocol.md](./stage-1-module-dashboard-protocol.md) | 无 |
| 2 | `acc-coach` | [stage-2-acc-coach.md](./stage-2-acc-coach.md) | Stage 1 |
| 3 | `module_local_dashboard` | [stage-3-module-local-dashboard.md](./stage-3-module-local-dashboard.md) | Stage 2（可 mock 部分并行） |
| 4 | 联调测试 | [stage-4-integration-test.md](./stage-4-integration-test.md) | Stage 1–3 |

## 并行开发说明

- Stage 1 改动极小（一个 struct + 一个字段 + 常量），建议先完成再启动 Stage 2，避免 mock/对齐返工。
- Stage 2 和 Stage 3 可部分并行：`module_local_dashboard` 可先写 TS 端的 binding 读取逻辑 + `controlDependencies` 改动，并用 mock 数据验证 UI 行为，等 `acc-coach` 的编译产物真实可用后再联调。
- Stage 4 不可并行：必须在三个模块改动都完成后进行端到端验证。

## 不变的部分

- 坐标变换逻辑保持不变：Map Widget 对 `carX`/`carZ` 应用 `angleDeg`/`flipX`/`flipZ` 后与赛道点一起绘制。
- 现有 `MapWidget` 的 track 加载、canvas 绘制逻辑保持不变。
- `telemetry_fields_for_control` 职责不变（只管文本/模板/图表/条件规则字段），bindings 订阅是并行的第二条来源。
