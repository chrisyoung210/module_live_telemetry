# 阶段 3：前端渲染优化

日期：2026-06-29
涉及模块：`module_local_dashboard`（仅此模块）
依赖：阶段2（event push 基础上做 store，但也可在 polling 基础上独立做）

---

## 1. 目标

优化前端渲染路径，降低 React 渲染成本，使 50 控件场景下 JS 主线程稳定，高频字段更新不拖累低频控件。

**预期效果**：单帧 JS 处理 < 4ms，50 控件 60Hz 稳定。

## 2. 当前问题

### 2.1 useState 触发全量渲染

`useDashboardFrame.ts:61`：

```typescript
setFullFrame({
    sampleTick: frame.sampleTick,
    timestampNs: frame.timestampNs,
    values: { ...fullFrameRef.current },  // 每次创建新对象
});
setHistoryVersion((v) => v + 1);
```

每次收到帧创建新对象 + 两个 state 更新 → 整个 `LocalDashboardOverlay` 重新 render → 所有 `DashboardRegionRenderer` 重新 render。

### 2.2 所有控件共享 frame prop

`dashboardRenderer.tsx:155-165`：

```tsx
{layout.controls.map((control) => (
    <DynamicDashboardControl
        key={control.id}
        control={control}
        frame={frame}              // 所有控件共享同一 frame
        historyBuffer={historyBuffer}
        historyVersion={historyVersion}
        ...
    />
))}
```

虽然 `DynamicDashboardControl` 有 `memo` + `controlPropsAreEqual`，但每次 `frame` 都是新对象引用，所有控件的 memo 比较都要执行。

### 2.3 controlPropsAreEqual 依赖字符串 key 查找

`dashboardRenderer.tsx:219-250`：

```typescript
function controlPropsAreEqual(previous, next): boolean {
    // ...
    return controlFrameInputsAreEqual(
        next.control,
        previous.frame,
        next.frame,
        next.gearState !== undefined,
    );
}

function controlFrameInputsAreEqual(control, previous, next, usesGearSmoother): boolean {
    const dependencies = controlDependencies(control);
    for (const field of dependencies) {
        if (!Object.is(prevValues[field], nextValues[field])) return false;
    }
    // ...
}
```

每次比较都要 `controlDependencies(control)`（虽有 WeakMap 缓存）+ 遍历依赖字段做 string key 查找。

## 3. 改动方案

### 3.1 C3：useSyncExternalStore 替代 useState

**改动文件**：`src-ui/features/local-dashboard-overlay/useDashboardFrame.ts`

引入外部 mutable store，避免 React state 快照语义的额外拷贝和全量渲染。

```typescript
import { useSyncExternalStore } from "react";

// External store — 不触发 React state 更新，订阅者自行决定是否重渲染
class DashboardFrameStore {
    private frame: DashboardValuesFrame | null = null;
    private historyVersion = 0;
    private historyRef: Map<string, BufferEntry[]> = new Map();
    private listeners: Set<() => void> = new Set();

    getFrame = (): DashboardValuesFrame | null => this.frame;
    getHistoryVersion = (): number => this.historyVersion;
    getHistoryBuffer = (): Map<string, BufferEntry[]> => this.historyRef;

    subscribe = (listener: () => void): (() => void) => {
        this.listeners.add(listener);
        return () => this.listeners.delete(listener);
    };

    setFrame(frame: DashboardValuesFrame) {
        // 合并稀疏帧
        // ... existing merge logic ...
        this.frame = { /* ... */ };
        this.historyVersion++;
        this.listeners.forEach(l => l());
    }

    clear() {
        this.frame = null;
        this.historyRef.clear();
        this.historyVersion++;
        this.listeners.forEach(l => l());
    }
}

// 全局单例（或 per-overlay 实例）
const store = new DashboardFrameStore();

export function useDashboardFrame(frameMs: number = 16) {
    const frame = useSyncExternalStore(store.subscribe, store.getFrame);
    const historyVersion = useSyncExternalStore(store.subscribe, store.getHistoryVersion);
    const historyBuffer = store.getHistoryBuffer();  // stable ref

    // event listener / polling 逻辑调用 store.setFrame / store.clear
    // ... event/polling setup ...

    return { fullFrame: frame, historyBuffer, historyVersion, rebuildBuffers };
}
```

**收益**：
- 避免 `setFullFrame` 的对象创建
- `useSyncExternalStore` 与 React 并发渲染更好配合
- store 是 mutable 的，不每次创建新对象

### 3.2 C4：控件级字段选择器

**改动文件**：`src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx`

每个控件只订阅自己依赖的字段，而非整个 frame。

#### 方案

引入 selector hook：

```typescript
function useControlFieldValue(
    control: DashboardControl,
    store: DashboardFrameStore,
): { frame: DashboardValuesFrame | null; shouldRender: boolean } {
    const dependencies = useMemo(() => controlDependencies(control), [control]);
    const fieldValues = useSyncExternalStore(
        store.subscribe,
        () => {
            const frame = store.getFrame();
            if (!frame) return null;
            // 只提取依赖字段的值，用于比较
            const snapshot: Record<string, number | undefined> = {};
            for (const field of dependencies) {
                snapshot[field] = frame.values[field];
            }
            return JSON.stringify(snapshot);  // 或自定义比较
        },
    );
    // ...
}
```

**更高效的方案**：用自定义比较函数而非 JSON.stringify：

```typescript
// DashboardFrameStore 增加 per-field version 机制
class DashboardFrameStore {
    private fieldVersions: Map<string, number> = new Map();
    private globalVersion = 0;

    setFrame(frame: DashboardValuesFrame) {
        for (const field of Object.keys(frame.values)) {
            this.fieldVersions.set(field, (this.fieldVersions.get(field) ?? 0) + 1);
        }
        this.globalVersion++;
        this.listeners.forEach(l => l());
    }

    getFieldVersion(field: string): number {
        return this.fieldVersions.get(field) ?? 0;
    }
}

// 控件级 selector
function useControlDependenciesChanged(
    dependencies: string[],
    store: DashboardFrameStore,
): boolean {
    const lastVersions = useRef<number[]>([]);
    const currentVersions = dependencies.map(d => store.getFieldVersion(d));
    const changed = currentVersions.some((v, i) => v !== lastVersions.current[i]);
    if (changed) lastVersions.current = currentVersions;
    return changed;
}
```

#### DynamicDashboardControl 改造

```tsx
export const DynamicDashboardControl = memo(function DynamicDashboardControl({
    control,
    store,            // 替代 frame prop
    historyBuffer,
    historyVersion,
    trackPoints,
    gearState,
}: DynamicDashboardControlProps) {
    const dependencies = useMemo(() => controlDependencies(control), [control]);
    const depsChanged = useControlDependenciesChanged(dependencies, store);

    // 从 store 读取当前 frame（stable ref，不会因无关字段变化而变）
    const frame = useSyncExternalStore(store.subscribe, store.getFrame);

    // 如果依赖字段未变化，跳过渲染
    if (!depsChanged && /* control 未变 */) return null;  // 或缓存的上次渲染结果

    const widgetType = resolveWidgetType(control);
    switch (widgetType) {
        case "chart": return <ChartWidget control={control} historyBuffer={historyBuffer} historyVersion={historyVersion} />;
        case "map": return <MapWidget control={control} frame={frame} trackPoints={trackPoints} />;
        case "text":
        default: return <TextWidget control={control} frame={frame} gearState={gearState} />;
    }
});
```

**收益**：
- rpm 120Hz 更新只触发依赖 rpm 的控件重渲染
- lap-time 10Hz 控件不被 rpm 高频更新拖累
- 50 控件场景下 JS 渲染成本稳定

## 4. 模块间开发顺序

本阶段仅涉及 `module_local_dashboard`，无跨模块协调。

建议开发顺序：
1. C3（useSyncExternalStore）— 先改 store 层，保持组件接口不变
2. C4（控件级选择器）— 在 store 基础上改控件订阅

## 5. 验收标准

| 验收项 | 验证方法 |
|---|---|
| overlay 正常显示 | 启动 ACC + recording，观察 overlay |
| 50 控件稳定 | 创建 50 控件 layout，观察渲染不卡顿 |
| 高频不拖累低频 | rpm 120Hz + lap-time 10Hz，lap-time 控件不被 rpm 拖累 |
| React Profiler | DevTools Profiler 观察渲染频率，控件只在自己依赖字段变化时渲染 |
| 暂停/恢复 | overlay 状态正确 |
| 布局切换 | 切换 layout 后无 stale value |
| chart 正常 | ChartWidget 从缓冲区渲染正常 |

## 6. 风险

| 风险 | 缓解 |
|---|---|
| `useSyncExternalStore` 在 Tauri webview 中的兼容性 | Tauri 使用系统 webview，React 18+ 的 useSyncExternalStore 在 Chromium/WebView2 中已稳定 |
| per-field version 机制内存增长 | `fieldVersions` Map 在 layout 切换时清理；字段数量有限（<100） |
| 控件 memo 破坏 layout 切换 | layout 切换时 control 引用变化，memo 自动失效 |

## 7. 参照文档

- `docs/acc-coach/2026-06-16-dashboard-display-performance-plan.md` — 阶段4节"前端状态与渲染优化"
- `docs/acc-coach/prd/local-dashboard-self-managed-data.md` — FR-04 帧轮询与合并逻辑
- README.md 关键代码位置索引 — module_local_dashboard `DynamicDashboardControl` / `controlDependencies` 条目
