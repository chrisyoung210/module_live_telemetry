# module_dashboard_protocol 类型对齐需求

> 目标模块：`module_dashboard_protocol`
>
> 需求方：`acc-coach`、`module_local_dashboard`
>
> 需求类型：类型结构调整（非新增功能）
>
> 状态：等待开发

---

## 一、背景

当前的 `module_dashboard_protocol/types/index.ts` 定义的 `DashboardControl` 接口与 `acc-coach` 的 Rust `DashboardControl` 结构体之间字段不一致，导致 overlay 加载布局数据时字段缺失，所有 widget 显示默认值 `"--"`。

核心问题：协议定义缺少字段、字段名不匹配、存在历史兼容代码包袱。

本次重构的目标：**以协议为尊**，协议定义完整的 `DashboardControl` 类型，消费者 (`acc-coach` Rust 端和 `module_local_dashboard`) 输出/读取协议标准格式。

---

## 二、现有类型对照

### 协议当前 `DashboardControl`（`types/index.ts:31`）

```typescript
export interface DashboardControl {
  id: string;
  widgetType: WidgetType;
  telemetryField?: string | null;
  textTemplate?: string | null;
  format?: string | null;
  refreshHz: number;
  x: number; y: number; width: number; height: number;
  font?: string | null;
  fontSize?: number | null;
  textColor?: string | null;
  backgroundColor?: string | null;
  chartFields: ChartFieldConfig[];
  chartSampleCount?: number | null;
  trackId?: string | null;
  dotColor?: string | null;
  dotSize?: number | null;
  conditionalRules: DashboardConditionalRule[];
}
```

### acc-coach 当前 Rust `DashboardControl`（`src/dashboard/layout.rs:43`）

Rust 结构体 serde 序列化为 `camelCase` JSON，有以下协议**缺失**的字段：

| 字段 | 当前 JSON 名 | 类型 | 说明 |
|------|-------------|------|------|
| `visible` | `visible` | `bool` | 控件是否可见 |
| `visible_expression` | `visibleExpression` | `string \| null` | 可见性动态表达式 |
| `background_color_expression` | `backgroundColorExpression` | `string \| null` | 背景色动态表达式 |
| `background_image_data_url` | `backgroundImageDataUrl` | `string \| null` | 控件背景图 |
| `text_fallback` | `textFallback` | `string \| null` | 文本无数据时的 fallback |
| `text_color_expression` | `textColorExpression` | `string \| null` | 文字色动态表达式 |
| `horizontal_center` | `horizontalCenter` | `bool` | 水平居中 |
| `vertical_center` | `verticalCenter` | `bool` | 垂直居中 |

现有字段的命名差异：

| Rust 字段 | Rust JSON 输出 | 协议字段 | 对齐方向 |
|-----------|---------------|----------|----------|
| `text` | `text` | `textTemplate` | Rust 改为输出 `textTemplate` |
| `text_format` | `textFormat` | `format` | Rust 改为输出 `format` |
| `font_family` | `fontFamily` | `font` | 协议 `font` 改为 `fontFamily` |

---

## 三、目标协议类型（TS 端）

```typescript
export interface DashboardControl {
  id: string;
  widgetType: WidgetType;

  // 数据绑定
  telemetryField: string | null;
  textTemplate: string;                    // 模板字符串，如 "{speedKmh} km/h"
  format: string | null;

  // 布局
  x: number;
  y: number;
  width: number;
  height: number;
  refreshHz: number;

  // 可见性
  visible: boolean;                        // 新增
  visibleExpression: string | null;        // 新增

  // 外观
  backgroundColor: string;                     // 必填，与 Rust 端 String 对齐（空串表示透明）
  backgroundColorExpression: string | null;  // 新增
  backgroundImageDataUrl: string | null;     // 新增
  fontFamily: string;                        // 原 font，改为必填
  fontSize: number;                          // 原可选，改为必填
  textColor: string;                         // 原可选，改为必填
  textColorExpression: string | null;        // 新增
  textFallback: string | null;              // 新增
  horizontalCenter: boolean;                 // 新增
  verticalCenter: boolean;                   // 新增

  // 条件规则
  conditionalRules: DashboardConditionalRule[];

  // Chart 专用
  chartFields: ChartFieldConfig[];
  chartSampleCount: number | null;

  // Map 专用
  trackId: string | null;
  dotColor: string | null;
  dotSize: number | null;
}
```

### `ChartFieldConfig` 字段重命名

```typescript
// 当前
export interface ChartFieldConfig {
  fieldName: string;   // ← 重命名
  color: string;
  label: string;
  defaultValue?: number | null;
}

// 目标
export interface ChartFieldConfig {
  telemetryField: string;  // 与 DashboardControl.telemetryField 一致
  color: string;
  label: string;
  defaultValue?: number | null;
}
```

---

## 四、目标协议类型（Rust 端）

协议 Rust crate（`module_dashboard_protocol/src/lib.rs`）的 `DashboardControl`（`lib.rs:78`）必须与上节 TS 端逐字段对齐。当前 Rust 端同样缺失 8 个字段、`ChartFieldConfig.field_name`（`lib.rs:42`）需重命名、`font`（`lib.rs:113`）需改为 `font_family`。**仅改 TS 端而 Rust 端不动，会导致协议两端类型不一致，acc-coach 也无法 re-export 协议 Rust 类型。**

### 4.1 `DashboardControl` 结构体改动

| 改动类型 | Rust 字段 | 序列化 JSON 名 | 说明 |
|----------|-----------|---------------|------|
| 新增 | `visible` | `visible` | `bool`，`#[serde(default = "default_control_visible")]` |
| 新增 | `visible_expression` | `visibleExpression` | `Option<String>` |
| 新增 | `background_color_expression` | `backgroundColorExpression` | `Option<String>` |
| 新增 | `background_image_data_url` | `backgroundImageDataUrl` | `Option<String>` |
| 新增 | `text_fallback` | `textFallback` | `Option<String>` |
| 新增 | `text_color_expression` | `textColorExpression` | `Option<String>` |
| 新增 | `horizontal_center` | `horizontalCenter` | `bool` |
| 新增 | `vertical_center` | `verticalCenter` | `bool` |
| 重命名 | `font` → `font_family` | `fontFamily` | 类型由 `Option<String>` 改为 `String`（必填） |
| 必填化 | `font_size` | `fontSize` | `Option<f64>` → `f64` |
| 必填化 | `text_color` | `textColor` | `Option<String>` → `String` |
| 必填化 | `background_color` | `backgroundColor` | `Option<String>` → `String` |

> `text_template` / `format` / `telemetry_field` 在协议 Rust 端已正确（序列化为 `textTemplate` / `format` / `telemetryField`），无需改动字段名，仅需将 `text_template` 由 `Option<String>` 调整为 `String`（与 TS 端一致，必填）。

### 4.2 自定义 Deserialize 同步更新

`DashboardControl` 的自定义 `Deserialize` impl（`lib.rs:141`）中的 `Helper` 结构体需同步补齐上述新增字段，并将 `font` 字段重命名为 `font_family`。`isDynamic` → `widgetType` 的兼容映射可保留（与 TS 端 `normalizeWidgetType` 的删除不冲突——`normalizeWidgetType` 是运行时 JS 兼容函数，Rust 端的 `isDynamic` 反序列化兼容属协议内部行为，本 PRD 不要求删除）。

### 4.3 `ChartFieldConfig` 字段重命名

```rust
// 当前（lib.rs:42）
pub struct ChartFieldConfig {
    pub field_name: String,   // 序列化为 fieldName
    // ...
}

// 目标
pub struct ChartFieldConfig {
    pub telemetry_field: String,  // 序列化为 telemetryField，与 DashboardControl.telemetryField 一致
    // ...
}
```

### 4.4 测试更新

协议 Rust crate 的单元测试（`lib.rs:392` 起 `mod tests`）中：
- `chart_field_config_roundtrip`（`lib.rs:468`）的 `field_name` 字段访问改为 `telemetry_field`
- `layout_payload_roundtrip`（`lib.rs:585`）等用例补齐新增字段赋值
- `control_minimal_json_uses_defaults`（`lib.rs:730`）的断言补齐新增字段默认值校验

---

## 五、需要删除的内容

以下函数/类型是历史兼容代码，本次删除，不做兼容：

1. **`normalizeWidgetType()`**（第 114 行）— `isDynamic` 到 `widgetType` 的旧版映射，已无使用者需要
2. **`normalizeLayoutPayload()`**（第 131 行）— 合并 `staticControls` + `dynamicControls` 的逻辑，已由 Rust 侧 `DashboardLayoutPayload::serialize()` 处理
3. **`normalizeControl()`**（第 158 行）— snake_case / camelCase 回退兼容，字段统一后不再需要

acc-coach 和 module_local_dashboard 将在本需求实现后**同步删除对这些函数的引用**。

---

## 六、acc-coach Rust 类型归属

acc-coach 当前在 `src/dashboard/layout.rs:43` 自定义了 `DashboardControl` 结构体及其自定义 `Deserialize`（`layout.rs:84`），与协议 Rust crate 的同名结构体并存，是字段漂移的根源之一。本次定调：

**决策**：协议 Rust 端按第四节补齐字段后，acc-coach 删除 `layout.rs:43` 的自有 `DashboardControl` 定义及其自定义 `Deserialize`，改为：

```rust
// src/dashboard/layout.rs
pub use module_dashboard_protocol::{
    ChartFieldConfig, DashboardConditionalRule, DashboardControl,
    DashboardTextFormat, DashboardTextFormatOrRaw, DashboardValuesFrame, WidgetType,
};
```

**理由**：
1. 落实"协议为尊"原则，消除 acc-coach 与协议两份结构体并存的双源维护
2. `layout.rs:10-13` 已 re-export `WidgetType`/`DashboardValuesFrame` 等次要类型，`src/dashboard/output.rs:12` 的 `TODO(mdp)` 注释也预设此方向
3. 协议 Rust 端的自定义 `Deserialize` 已处理 `isDynamic` 兼容（`lib.rs:141`），acc-coach 无需保留自己的兼容层

**前提**：协议 Rust 端必须先按第四节补齐 acc-coach 所需全部字段（`visible`/`textFallback`/`horizontalCenter`/`verticalCenter`/`backgroundColorExpression` 等），否则 acc-coach 现有功能字段会丢失。本 PRD 第四节已列明，是本决策的硬前置。

**保留不动**：acc-coach `layout.rs` 的 `DashboardDesignConfig`（`layout.rs:25`）、`DashboardFontAsset`（`layout.rs:34`）等自有类型与协议无关，继续保留。

---

## 七、acc-coach 侧连带清理

删除协议的 `normalizeWidgetType` / `normalizeLayoutPayload` / `normalizeControl`（见第五节）后，acc-coach 内部以下位置会编译失败，必须同步清理（这些不在 `module-local-dashboard-cleanup.md` 覆盖范围，因后者针对 overlay 子模块）：

| 文件 | 位置 | 当前引用 | 处理方式 |
|------|------|----------|----------|
| `src-ui/types.ts` | 第 4 行 | `export { normalizeWidgetType } from "@dashboard-protocol"` | 删除该 re-export |
| `src-ui/components/DashboardDesignerView.tsx` | 第 17 行 | `import { normalizeWidgetType } from "../types"` | 删除该 import |
| `src-ui/components/DashboardDesignerView.tsx` | 第 234 行 `normalizeControl()` | 函数内调用 `normalizeWidgetType(control as unknown as Record<string, unknown>)` | 协议类型对齐后 `widgetType` 已由协议保证，整个 `normalizeControl()` 函数可删除或简化为透传 |
| `src-ui/types.test.ts` | 第 2 行 import 及全部 `normalizeWidgetType` 测试用例 | 多处 | 删除相关测试用例 |
| `src-ui/components/DashboardDesignerView.test.tsx` | 第 2 行 import 及 `normalizeWidgetType` 测试用例 | 多处 | 删除相关测试用例 |

> 注：`normalizeControl()`（`DashboardDesignerView.tsx:234`）除调用 `normalizeWidgetType` 外，还做 `refreshHz` 默认值、`backgroundColor`/`textColor` 的 `normalizeArgb` 等归一化。删除前需确认这些归一化逻辑是否仍有必要保留；若需要，将其拆出为独立函数，不要连带删除。

---

## 八、接口契约

协议 vNext 发布后，消费者需遵守：

1. **acc-coach Rust 端**：删除 `src/dashboard/layout.rs` 自有的 `DashboardControl`，改为 `pub use module_dashboard_protocol::DashboardControl`（见第六节），序列化字段名自然与协议一致
2. **acc-coach 前端**：删除 `src-ui/types.ts` 中重复的 `DashboardControl` 定义，改为 `export type { DashboardControl } from "@dashboard-protocol"`
3. **module_local_dashboard**：`resolveControlText()` 直接读取 `control.textTemplate` / `control.telemetryField` / `control.format`，不做蛇形/驼峰回退

---

## 九、与其他 PRD 的依赖关系

- 本 PRD 为先决条件，所有其他模块的类型对齐依赖此协议发布
- 无依赖其他 PRD

---

## 十、验收标准

1. 协议 TS 端 `DashboardControl` 包含所有上述字段，缺失的字段已补齐
2. 协议 Rust 端 `DashboardControl` 同步补齐 8 个新增字段，`font` 重命名为 `font_family`，`text_template`/`font_size`/`text_color`/`background_color` 调整为必填
3. 协议 `ChartFieldConfig.fieldName`（TS）/ `field_name`（Rust）重命名为 `telemetryField` / `telemetry_field`
4. `normalizeWidgetType`、`normalizeLayoutPayload`、`normalizeControl` 三个函数已删除
5. 协议类型文件仅包含纯类型定义，无运行时转换逻辑
6. acc-coach Rust 端改为 `pub use module_dashboard_protocol::DashboardControl`，`src/dashboard/layout.rs` 自有定义与自定义 `Deserialize` 已删除
7. acc-coach 前端 `normalizeWidgetType` 的 re-export、import 及相关测试用例已清理
