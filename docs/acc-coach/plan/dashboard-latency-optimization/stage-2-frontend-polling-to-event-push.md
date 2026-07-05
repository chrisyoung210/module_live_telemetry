# 阶段 2：前端 polling → event push

日期：2026-06-29
涉及模块：`acc-coach`（event 发送）+ `module_local_dashboard`（event 监听 + 渲染）
依赖：无（可与阶段1并行，但建议在阶段1完成后做，因为阶段1的 LatestValueSink + Select 更容易接入 event emit）

---

## 1. 目标

消除前端 setInterval polling 的 0~33ms 等待延迟，改为后端有新数据时主动推送事件，前端 `requestAnimationFrame` 合帧渲染。保留 polling fallback 保证兼容性。

**预期效果**：前端数据到达延迟从平均 17ms 降到 0~2ms。

## 2. 当前问题

### 2.1 setInterval polling

`module_local_dashboard/src-ui/features/local-dashboard-overlay/useDashboardFrame.ts:94-121`：

```typescript
const poll = async () => {
    const frame = await invoke<DashboardValuesFrame | null>("poll_dashboard_frame");
    if (frame) { pushFrame(frame); } else { handleClear(); }
};
poll();
intervalRef.current = setInterval(poll, frameMs);  // frameMs 默认 33ms
```

问题：
- 新数据刚错过一次 poll，需等下一轮（0~33ms）
- IPC 调用有序列化、调度、跨线程成本
- 上一轮 IPC 未返回时无法发起新轮（in-flight 隐含防重入）
- 即使 `frameMs` 调到 8ms，setInterval 精度也不够

### 2.2 默认 frameMs=33

`module_local_dashboard/src/local_dashboard_overlay/config.rs:81`：

```rust
fn default() -> Self {
    Self { status_ms: 500, frame_ms: 33, window_ms: 500 }
}
```

33ms ≈ 30Hz polling，与后端 120Hz 产出不匹配。

## 3. 改动方案

### 3.1 C1：event push

#### 3.1.1 acc-coach 侧：emit dashboard frame event

**改动文件**：`src/recording/auto.rs`

在 `auto_recording_loop` 的 dashboard 帧 drain 完成后，向 overlay window emit 事件。

**前提**：`auto_recording_loop` 需要获取 `tauri::AppHandle` 以调用 `app.emit()`。当前 `auto_recording_loop` 是独立线程，不持有 AppHandle。需要在 `AutoRecordingMonitor::start` / `spawn_runtime` 中传入 `AppHandle`。

**具体改动**：

1. `AutoRecordingMonitor::start` 和 `start_with_telemetry` 增加 `app_handle: tauri::AppHandle` 参数
2. `spawn_runtime` 将 `app_handle` 传入 `auto_recording_loop`
3. `auto_recording_loop` 在 drain 完成后 emit：

```rust
// drain 完成后
if frame_count > 0 {
    push_merged_to_bus(&latest_dashboard_frame, &bus, &alias);
    // emit event 给 overlay window
    if let Some(frame) = latest_dashboard_frame.lock().ok().as_ref().and_then(|f| f.as_ref()) {
        let payload = LiveDashboardFrame {
            subscription_generation: 0,
            sample_tick: frame.sample_tick,
            timestamp_ns: frame.timestamp_ns,
            values: frame.values.clone(),  // raw key（阶段1 A4 后 bus 存 raw key，event 也发 raw key）
        };
        // 只发给 overlay window，不给主窗口
        use tauri::Emitter;
        let _ = app_handle.emit_to("local-dashboard-overlay", "dashboard://frame", &payload);
    }
}
```

**事件名称**：`dashboard://frame`

**事件 payload**：与 `poll_dashboard_frame` IPC 返回的 `LiveDashboardFrame` 相同结构（raw key，前端收到后自行翻译或直接使用——当前前端通过 IPC 收到的也是翻译后的 key，所以 event 也需要翻译）。

**注意**：如果阶段1 A4 已完成（alias 翻译移到 IPC），event payload 应在 emit 前做 alias 翻译。如果阶段1未完成，event payload 保持 raw key 并由前端处理（但前端当前期望 user-facing key，所以需要 emit 前翻译）。

**推荐**：在 emit 前做 alias 翻译，使 event payload 与 `poll_dashboard_frame` 返回值一致：

```rust
// emit 前翻译
let mut translated_values = frame.values.clone();
translate_dashboard_frame_values(&mut translated_values, &alias);
let _ = app_handle.emit_to("local-dashboard-overlay", "dashboard://frame", &LiveDashboardFrame {
    subscription_generation: 0,
    sample_tick: frame.sample_tick,
    timestamp_ns: frame.timestamp_ns,
    values: translated_values,
});
```

4. `AutoRecordingMonitor::start` 调用方（`src/lib.rs` 或 `src/ipc/mod.rs` setup）传入 `app_handle`

5. overlay window label 确认：需确认 overlay window 的 label（当前可能是 `local-dashboard-overlay` 或其他名称，需检查 `tauri.conf.json` 或窗口创建代码）

#### 3.1.2 module_local_dashboard 侧：listen + rAF 合帧

**改动文件**：`src-ui/features/local-dashboard-overlay/useDashboardFrame.ts`

新增 Tauri event listener，收到事件后用 `requestAnimationFrame` 合帧：

```typescript
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export function useDashboardFrame(frameMs: number = 16) {
    const [fullFrame, setFullFrame] = useState<DashboardValuesFrame | null>(null);
    const [historyVersion, setHistoryVersion] = useState(0);
    // ... refs 保持不变 ...

    // pending frame buffer for rAF coalescing
    const pendingFrameRef = useRef<DashboardValuesFrame | null>(null);
    const rafScheduledRef = useRef(false);

    // flush pending frame on next animation frame
    const flushPendingFrame = useCallback(() => {
        rafScheduledRef.current = false;
        const frame = pendingFrameRef.current;
        if (!frame) return;
        pendingFrameRef.current = null;
        pushFrame(frame);  // 复用现有 pushFrame 逻辑
    }, [pushFrame]);

    // event-driven path
    useEffect(() => {
        let unlisten: (() => void) | null = null;
        let listenFailed = false;

        listen<DashboardValuesFrame>("dashboard://frame", (event) => {
            const frame = event.payload;
            if (frame.values && Object.keys(frame.values).length > 0) {
                pendingFrameRef.current = frame;
                if (!rafScheduledRef.current) {
                    rafScheduledRef.current = true;
                    requestAnimationFrame(flushPendingFrame);
                }
            } else if (frame === null) {
                handleClear();
            }
        }).then((fn) => {
            unlisten = fn;
        }).catch(() => {
            listenFailed = true;
            // listen 失败时 fallback 到 polling（下方 useEffect 会启动 polling）
        });

        return () => {
            if (unlisten) unlisten();
        };
    }, [flushPendingFrame, handleClear]);

    // polling fallback（event 失败或未就绪时）
    useEffect(() => {
        const poll = async () => {
            try {
                const frame = await invoke<DashboardValuesFrame | null>("poll_dashboard_frame");
                if (frame) {
                    pendingFrameRef.current = frame;
                    if (!rafScheduledRef.current) {
                        rafScheduledRef.current = true;
                        requestAnimationFrame(flushPendingFrame);
                    }
                } else {
                    handleClear();
                }
            } catch { /* IPC not ready */ }
        };

        // 延迟启动 polling，给 event listener 时间初始化
        const timer = setTimeout(() => {
            poll();
            intervalRef.current = setInterval(poll, frameMs);
        }, 200);  // 200ms 后如果 event 工作则 polling 只做 fallback

        return () => {
            clearTimeout(timer);
            if (intervalRef.current) {
                clearInterval(intervalRef.current);
                intervalRef.current = null;
            }
        };
    }, [flushPendingFrame, handleClear, frameMs]);

    return { fullFrame, historyBuffer: historyRef.current, historyVersion, rebuildBuffers };
}
```

**关键设计**：
- event listener 和 polling 都写入 `pendingFrameRef`，rAF 统一 flush
- 如果 event 工作正常，polling 每 frameMs 也跑一次但通常读到相同数据（bus 存最新帧），不会产生重复渲染（rAF 合帧保证）
- 如果 event 失败（acc-coach 未注册 listener 或版本不匹配），polling 保证功能正常
- 200ms 延迟启动 polling 是为了给 event listener 初始化时间，避免双重数据流

#### 3.1.3 overlay window label 确认

需确认 overlay window 的 Tauri label。查看窗口创建代码：

`acc-coach/src-ui/components/LocalDashboardOverlayWindow.tsx` 或 `module_local_dashboard/src/lib.rs` 中的窗口创建逻辑。

`emit_to(label, event, payload)` 中的 `label` 必须与窗口创建时的 label 一致。如果不确定，可用 `app.emit(event, payload)` 广播给所有窗口（主窗口会收到但忽略），或检查 `tauri.conf.json`。

### 3.2 C2：降低 frameMs 默认值

**改动文件**：`module_local_dashboard/src/local_dashboard_overlay/config.rs`

```rust
fn default() -> Self {
    Self {
        status_ms: 500,
        frame_ms: 16,   // 33 → 16，约 60Hz polling fallback
        window_ms: 500,
    }
}
```

`normalized` 的 clamp 范围（`config.rs:117`）已经是 `4..=1000`，不需要改。

**注意**：这是 module_local_dashboard 的改动，需要告知用户在独立会话中完成。

## 4. 模块间开发顺序

本阶段涉及两个模块，可并行开发：

### 并行开发

| 模块 | 改动 | 并行可行性 |
|---|---|---|
| acc-coach | 加 `app_handle` 传参 + `emit_to("dashboard://frame")` | 独立，不破坏现有 polling |
| module_local_dashboard | 加 `listen("dashboard://frame")` + rAF 合帧 + frameMs 默认值 | 独立，不破坏现有 polling（listen 失败时 fallback） |

### 联调点

两边都完成后联调：
1. 确认 event 名称一致：`dashboard://frame`
2. 确认 overlay window label 一致
3. 确认 payload 结构一致（`LiveDashboardFrame` / `DashboardValuesFrame`）
4. 确认 event payload 的 key 格式与 polling 返回值一致（都是 user-facing key）

### 不破坏现有功能

- acc-coach 加 emit 不影响现有 `poll_dashboard_frame` IPC，polling 继续工作
- module_local_dashboard 加 listen 不影响现有 setInterval，listen 失败时 polling fallback
- 两边任何一方未改完，功能都正常（只是没有 event push 优化）

## 5. 验收标准

| 验收项 | 验证方法 |
|---|---|
| overlay 正常显示 | 启动 ACC + recording，观察 overlay |
| event push 工作 | 浏览器 DevTools 中 `listen` 成功，Console 无 listen 错误 |
| 延迟降低 | 录屏暂停对比，gap 明显缩小 |
| polling fallback | 临时禁用 event emit（或旧版 acc-coach），overlay 仍正常 |
| rAF 合帧 | 后端 120Hz 时前端不卡顿，CPU 正常 |
| 无重复渲染 | DevTools React Profiler 观察渲染频率 ≈ 显示器刷新率 |
| 暂停/恢复 | overlay 状态正确 |
| remote dashboard 不受影响 | remote 仍通过 telemetry_tx 收帧 |

## 6. 风险

| 风险 | 缓解 |
|---|---|
| Tauri event emit 高频 overhead | 先实测 120Hz emit 的 CPU 开销；如过高可加节流（如限制 emit 频率到 60Hz） |
| `app_handle` 传参改动面较大 | `AutoRecordingMonitor::start` 调用方少，改动可控 |
| overlay window label 不匹配 | 用 `app.emit`（广播）替代 `emit_to` 作为 fallback |
| event listener 初始化慢 | 200ms 延迟启动 polling fallback |
| rAF 在窗口隐藏时暂停 | 窗口隐藏时 overlay 不需要渲染，polling 也不需要——与现有 visible 状态控制一致 |

## 7. 参照文档

- `docs/acc-coach/2026-06-16-dashboard-display-performance-plan.md` — 阶段3节"IPC与推送模型优化"分析了 polling vs push
- `docs/acc-coach/public-protocol/dashboard-frame-distribution-protocol.md` — 当前帧分发协议
- README.md 关键代码位置索引 — module_local_dashboard `useDashboardFrame` 条目
