# Stage 4：联调测试

> **前置依赖**：Stage 1–3 全部完成
> **本 Stage 产物**：端到端验证通过

## 目标

验证三个模块改动协同后，field ID 在编译产物、telemetry 订阅、V2 frame、overlay 渲染各环节对齐，红点正常显示并随赛车移动。

## 端到端验证点

1. **编译产物**：`list_registered_dashboard_layouts` 返回的 map control JSON 包含 `bindings: [{role: "carX", fieldId: N}, {role: "carZ", fieldId: M}]`。
2. **telemetry 订阅**：writer 对含 Map widget 的 layout 订阅了 `calc:car_x` + `calc:car_z`。
3. **V2 frame**：`poll_dashboard_frame` / `dashboard://frame` 事件中的 frame 包含这两个 fieldId 的数值。
4. **红点显示**：Overlay 中 Map Widget 成功读取并绘制红点。
5. **红点实时跟随**：赛车移动时红点实时更新（不只是首帧画出）——验证 `controlDependencies` 含 bindings fieldId 后 `useSyncExternalStore` 链路通畅。
6. **回归**：非 Map widget 的 `bindings` 为空 `[]`，现有 text / chart 渲染不受影响。
7. **旧布局兼容**：`bindings` 字段使用 `#[serde(default)]`，旧 JSON 反序列化为空 vec，行为与升级前一致（不画红点），不报错。
8. **编译触发**：`list_registered_dashboard_layouts` 按需编译，升级后重新请求即生成带 `bindings` 的新布局。
