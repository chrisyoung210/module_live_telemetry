# Stage 3：module_local_dashboard — 依赖追踪与渲染

> **前置依赖**：Stage 2（编译产物含 bindings），可先用 mock 数据部分并行
> **本 Stage 产物**：types 更新、`controlDependencies` 修复、`MapWidget` 读取 bindings

## 目标

让 overlay 端从 `control.bindings` 读取 carX/carZ 的 fieldId，并确保 `useSyncExternalStore` 重渲染链路在 carX/carZ 变化时触发，红点随赛车实时移动。

## 改动清单

### 3.1 `src-ui/features/local-dashboard-overlay/types.ts`

- 根据更新后的 protocol 类型，补充 `CompiledFieldBinding` 和 `CompiledDashboardControl.bindings` 类型。`bindings` 定义为非可选（`CompiledFieldBinding[]`）。
- **直接删除** `MapWidgetFieldIds` 接口（不要标记 deprecated）。它是从未被填充的死代码，`dashboardRenderer.tsx:553` 的类型断言 `control as CompiledDashboardControl & MapWidgetFieldIds` 是当前 bug 的元凶之一。
- 同步删除 `dashboardRenderer.tsx` 中对 `MapWidgetFieldIds` 的 import。

### 3.2 `src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx`

#### 3.2.1（关键）更新 `controlDependencies`

`MapWidget` 的重渲染依赖 `useControlDepsVersion`（:184）→ `useSyncExternalStore`，而 `controlDependencies`（:288）只从 `compiledTextTemplate` 和 `conditionalRules` 提取 fieldId。

Map widget 模板通常为空或 `"{value}"`（且 `telemetryFieldId` 为 None），conditionalRules 为空 → dependencies 为空集 → getSnapshot 恒为 `""` → store 变化不触发重渲染 → **红点不随赛车移动，只在赛道数据加载/布局重编译时被动刷新**。

必须把 bindings 的 fieldId 加入依赖：

```ts
export function controlDependencies(control: CompiledDashboardControl): number[] {
  const cached = controlDependencyCache.get(control);
  if (cached) return cached;

  const dependencies = new Set<number>();
  // ... 原有 compiledTextTemplate / conditionalRules 逻辑不变 ...

  // bindings 的 fieldId 必须纳入依赖，否则 widget 不会因绑定字段变化而重渲染
  for (const b of control.bindings ?? []) {
    if (b.fieldId != null) dependencies.add(b.fieldId);
  }

  const result = [...dependencies];
  controlDependencyCache.set(control, result);
  return result;
}
```

#### 3.2.2 更新 `MapWidget` 读取 fieldId 的来源

改为从 `control.bindings` 读取（非可选链，类型已是非可选数组）：

```ts
const carXFieldId = control.bindings.find(b => b.role === ROLE_CAR_X)?.fieldId;
const carZFieldId = control.bindings.find(b => b.role === ROLE_CAR_Z)?.fieldId;

const carX = frame && carXFieldId != null ? frame.values.get(carXFieldId) : undefined;
const carZ = frame && carZFieldId != null ? frame.values.get(carZFieldId) : undefined;
```

- role 比较使用 `module_dashboard_protocol` 导出的常量（`ROLE_CAR_X` / `ROLE_CAR_Z`），不写裸字面量。
- 删除原 `const mapIds = control as CompiledDashboardControl & MapWidgetFieldIds;` 及后续 `mapIds.carXFieldId` / `mapIds.carZFieldId` 读取。
- 坐标变换逻辑（`angleDeg` / `flipX` / `flipZ`）、track 加载、canvas 绘制逻辑保持不变。

## 验证

- [ ] Map widget 成功读取 bindings 并绘制红点。
- [ ] **赛车移动时红点实时跟随**（不只是首帧画出）——确认 `controlDependencies` 包含 bindings fieldId 后 `useSyncExternalStore` 重渲染链路通畅。
- [ ] 回归：非 Map widget（text / chart）渲染不受影响。
