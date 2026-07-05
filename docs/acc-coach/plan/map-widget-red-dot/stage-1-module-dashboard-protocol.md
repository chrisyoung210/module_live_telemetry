# Stage 1：module_dashboard_protocol — 类型与常量定义

> **前置依赖**：无
> **本 Stage 产物**：`CompiledFieldBinding` 结构体、`CompiledDashboardControl.bindings` 字段、role 常量

## 目标

在协议层定义通用的 field binding 结构，不感知任何 widget 类型。为 acc-coach 编译期写入 fieldId、overlay 读取 fieldId 提供协议载体。

## 改动清单

文件：`module_dashboard_protocol/src/lib.rs`

### 1.1 新增 `CompiledFieldBinding` 结构体

```rust
#[serde(rename_all = "camelCase")]
pub struct CompiledFieldBinding {
    pub role: String,               // widget 内部语义，例如 "carX"
    pub field_id: DashboardFieldId, // FieldRegistry 数字 ID
}
```

**必须自带 `#[serde(rename_all = "camelCase")]`**：`CompiledDashboardControl` 的 camelCase rename 不会自动传递给嵌套 struct，缺失会导致 TS 端拿到 `field_id` 而非 `fieldId`。

### 1.2 在 `CompiledDashboardControl` 新增 `bindings` 字段

```rust
#[serde(default)]
pub bindings: Vec<CompiledFieldBinding>,
```

- `#[serde(default)]` 保证旧 JSON 反序列化为空 `vec`，向后兼容。
- TS 端类型为 `bindings: CompiledFieldBinding[]`（非可选）。JSON 中 `bindings` 始终出现且为 `[]` 或非空。

### 1.3 导出 role 常量

供 acc-coach（赋值方）和 `module_local_dashboard`（解释方）共同引用，避免拼写错误静默失效：

```rust
pub const ROLE_CAR_X: &str = "carX";
pub const ROLE_CAR_Z: &str = "carZ";
```

protocol 层只导出字符串常量，不引用 `WidgetType`，不破坏"不感知 widget 类型"原则。

### 1.4 更新测试构造点

`module_dashboard_protocol` 侧若有 `CompiledDashboardControl` 的直接构造 / Default / 示例，补充 `bindings: vec![]`。

> 注意：acc-coach 侧 `compile_control` 的测试通过 `DashboardControl`（未编译）+ `compile_control()` 间接产出 `CompiledDashboardControl`，不手动构造，不需要补 `bindings: vec![]`。

## 验证

- [ ] `module_dashboard_protocol` 编译通过。
- [ ] `ROLE_CAR_X` / `ROLE_CAR_Z` 常量可被外部 crate 导入。
- [ ] `CompiledDashboardControl` 的 serde 输出中 `bindings` 字段名为 camelCase（`fieldId`），且缺省时输出 `[]`。
