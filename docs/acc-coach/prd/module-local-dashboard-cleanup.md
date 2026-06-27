# module_local_dashboard 清理需求

> 目标模块：`module_local_dashboard`
>
> 需求方：`acc-coach`
>
> 需求类型：代码清理（删除冗余逻辑）
>
> 状态：等待开发
>
> 前置依赖：`module_dashboard_protocol` 类型对齐完成、`acc-coach` frame 输出统一

---

## 一、背景

当前 `module_local_dashboard` 内部维护了一份独立的通道别名表 (`fieldNameMap.ts`)，用于在 `frame.values` 中查找数据时做多 key 回退。这是因为 `acc-coach` 推送到 overlay 的 `DashboardValuesFrame.values` 中的 key 格式不统一——有时是规范格式 (`raw:controls.speed_kmh`)，有时是用户格式 (`speedKmh`)，有时甚至是 snake_case 原始名 (`speed_kmh`)。

同时，overlay 加载 layout 数据时还需调用 `normalizeLayoutPayload()` / `normalizeControl()` 做字段名蛇形/驼峰回退，因为 `acc-coach` 输出的 JSON 字段名与协议类型不完全一致。

经过 `acc-coach` 重构后：
- `frame.values` 的 key 统一为用户格式
- `layout.controls[].telemetryField` 统一为用户格式
- Rust `DashboardControl` 序列化输出完全对齐协议类型

因此 overlay 的这些兼容逻辑变为冗余代码，应予以清理。

---

## 二、需要删除的文件/代码

### 2.1 `src-ui/features/local-dashboard-overlay/fieldNameMap.ts` — 整体删除

该文件包含 `FIELD_ALIASES` 常量和 `resolveFieldValue()` / `resolveFieldKey()` 函数。这些将不再需要。

**当前不被清理但需注意**：文件被以下位置引用：
- `src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx` 的 `resolveControlText()`
- `src-ui/features/local-dashboard-overlay/useDashboardFrame.ts` 的数据绑定逻辑

### 2.2 `dashboardRenderer.tsx` — `resolveControlText()` 简化

**当前**（第 350-354 行）：
```typescript
let value = frame.values[field];
if (value === undefined) {
  const resolved = resolveFieldValue(field, frame.values);  // 回退查找
  if (resolved !== undefined) value = resolved;
}
```

**目标**：
```typescript
const value = frame.values[field];  // 直接查找，必定命中
if (value === undefined) return "--";
```

### 2.3 `useDashboardMetadata.ts` — 删除 `normalizeLayoutPayload` 调用

**当前**（第 56-60 行）：
```typescript
invoke<WireRegisteredDashboardLayout[]>("list_registered_dashboard_layouts"),
// ...
const allLayouts = (rawLayouts || []).map((wl) => ({
  ...wl,
  layout: normalizeLayoutPayload(wl.layout as unknown as Record<string, unknown>),
}));
```

**目标**：
```typescript
invoke<WireRegisteredDashboardLayout[]>("list_registered_dashboard_layouts"),
// ...
// 直接使用返回值，不做运行时转换
const allLayouts = rawLayouts || [];
```

### 2.4 `types.ts` — 删除 `normalizeLayoutPayload` / `normalizeWidgetType` 重新导出

**当前**（`src-ui/features/local-dashboard-overlay/types.ts`）：
```typescript
export { normalizeLayoutPayload, normalizeWidgetType } from "module_dashboard_protocol/types";
```
（这两个函数在协议侧已删除，此处引用应同步移除）

---

## 三、需要适配的变更

### 3.1 类型引用适配

协议 `DashboardControl` 新增字段：
- `fontFamily` 替代原 `font`
- `textTemplate` / `format` / `telemetryField` 字段保持不变
- 新增字段 (`visible`, `visibleExpression`, `horizontalCenter`, `verticalCenter` 等) 可能被 overlay 消费，但目前不需改动渲染逻辑

协议 `ChartFieldConfig.fieldName` → `telemetryField`：
- `dashboardRenderer.tsx` 第 484 行的 `field.telemetryField ?? field.telemetry_field` 简化为 `field.telemetryField`

> 注：本节引用的文件路径与行号位于外部子模块 `module_local_dashboard`，acc-coach 仓库内无法直接核对。实施前由 overlay 团队按本仓库当前 checkout 的子模块版本复核行号，并据此调整。

### 3.2 不再需要蛇形/驼峰回退

删除所有 `any_field ?? anyField` 模式（如 `field.telemetryField ?? field.telemetry_field`），因为 acc-coach 输出统一使用 camelCase。

---

## 四、验收标准

1. `fieldNameMap.ts` 文件已删除
2. `dashboardRenderer.tsx` 中无 `resolveFieldValue` / `resolveFieldKey` 调用
3. `useDashboardMetadata.ts` 中无 `normalizeLayoutPayload` 调用
4. `types.ts` 中无 `normalizeLayoutPayload` / `normalizeWidgetType` 重新导出
5. 无蛇形/驼峰回退模式
6. overlay 渲染功能正常：所有 widget 正确显示遥测数据（非 `"--"`）
