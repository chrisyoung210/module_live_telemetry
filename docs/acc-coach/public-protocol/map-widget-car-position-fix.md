# MapWidget 红点位置修复 — module_local_dashboard 改动说明

## 问题

Replay/Live 模式下赛道地图上的红点位置不正确。

## 根因：对车位置做了双重坐标变换

### 数据背景

`acc-coach` 的 Rust 端 `rotate_track_map` / `flip_track_map` 在用户旋转/翻转赛道时：

1. **已直接变换 points 坐标**（如 90° CW：`(x,z) → (z,-x)`，上下翻转：`z → -z`）
2. **同时更新** `angle_deg` / `flip_x` / `flip_z` 作为记录

因此 **DB 中存储的 points 已经是视觉上的最终坐标**。`angle_deg` / `flip_x` / `flip_z` 是"历史记录"字段，表示曾对 points 做过什么变换。

### 当前的渲染逻辑（有问题）

文件 `dashboardRenderer.tsx:594-615`：

```
normalizedCarPosition (0-1)
  → idx = round(carPos * (points.length - 1))
  → (x, z) = points[idx]              ← 取到的坐标已经是 Rust 变换后的
  → if angleDeg != 0: 对 (x,z) 做旋转  ← 又做了一次变换！
  → if flipX != -1: x = -x            ← 双重变换！
  → if flipZ != -1: z = -z
  → cx, cy = worldToScreen(...)
```

**points 已经被变换过，又从里面取坐标再变换一次，导致双重变换，红点位置错误。**

### 正确的坐标变换链

赛道轮廓绘制时直接使用 DB 中的 points 坐标（已变换），一步到位：

```
points[i] → worldToScreen → 赛道线绘制  ← 正确
```

红点应该走同样的路径——从**原始世界坐标**做一次变换：

```
carX, carZ (ACC 共享内存世界坐标)
  → 应用 angleDeg/flipX/flipZ   ← 一次变换
  → worldToScreen              ← 和赛道一样的映射
  → 红点绘制                   ← 正确
```

## 改动方案

### 改用 `carX` / `carZ` 替代 `normalizedCarPosition`

Rust 端已通过 `src/dashboard/output.rs:991-992` 将 ACC 共享内存的 `frame.car_x` / `frame.car_z` 作为 `carX` / `carZ` 放入 `DashboardValuesFrame.values`。`acc-coach` 端的 `MapWidget` 已改为直接使用 `carX`/`carZ` 绘制红点（不做任何坐标变换）。

### 具体代码改动

文件：`module_local_dashboard/src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx`

#### 1. 修改 telemetryField 读取（约 538-542 行）

**改前：**
```typescript
const telemetryField = control.telemetryField || "normalizedCarPosition";
const carPos =
  frame
    ? frame.values[telemetryField]
    : undefined;
```

**改后：**
```typescript
const carX =
  frame ? frame.values["carX"] : undefined;
const carZ =
  frame ? frame.values["carZ"] : undefined;
```

#### 2. 修改红点绘制逻辑（约 594-615 行）

**改前（双重变换）：**
```typescript
if (carPos !== undefined && Number.isFinite(carPos)) {
  const idx = Math.round(carPos * (points.length - 1));
  const clamped = Math.max(0, Math.min(points.length - 1, idx));
  let rCarX = points[clamped].x;
  let rCarZ = points[clamped].z;
  if (angleDeg !== 0) {
    const rad = (-angleDeg * Math.PI) / 180;
    const cos = Math.cos(rad), sin = Math.sin(rad);
    const cx = rCarX, cz = rCarZ;
    rCarX = cx * cos - cz * sin;
    rCarZ = cx * sin + cz * cos;
  }
  if (flipX !== 1.0) rCarX = -rCarX;
  if (flipZ !== 1.0) rCarZ = -rCarZ;
  const cx = (rCarX - minX) * scale + offsetX;
  const cy = -(rCarZ - maxZ) * scale + offsetY;

  ctx.fillStyle = dotColor ?? "#ff0";
  ctx.beginPath();
  ctx.arc(cx, cy, dotSize ?? 6, 0, Math.PI * 2);
  ctx.fill();
}
```

**改后（直接使用 `carX`/`carZ`，一次变换）：**
```typescript
if (carX !== undefined && Number.isFinite(carX) && carZ !== undefined && Number.isFinite(carZ)) {
  let rCarX = carX as number;
  let rCarZ = carZ as number;
  // 对原始世界坐标应用 track 的旋转变换（和 points 已应用的变换规则一致）
  if (angleDeg !== 0) {
    const rad = (-angleDeg * Math.PI) / 180;
    const cos = Math.cos(rad), sin = Math.sin(rad);
    const cx = rCarX, cz = rCarZ;
    rCarX = cx * cos - cz * sin;
    rCarZ = cx * sin + cz * cos;
  }
  if (flipX !== 1.0) rCarX = -rCarX;
  if (flipZ !== 1.0) rCarZ = -rCarZ;

  // clamp 到 track 包围盒内，防止红点超出赛道范围
  rCarX = Math.max(minX, Math.min(maxX, rCarX));
  rCarZ = Math.max(minZ, Math.min(maxZ, rCarZ));

  const cx = (rCarX - minX) * scale + offsetX;
  const cy = -(rCarZ - maxZ) * scale + offsetY;

  ctx.fillStyle = dotColor ?? "#ff0";
  ctx.beginPath();
  ctx.arc(cx, cy, dotSize ?? 6, 0, Math.PI * 2);
  ctx.fill();
}
```

#### 3. 更新 useEffect 依赖（约 616 行）

**改前：**
```typescript
}, [points, carPos, width, height, dotColor, dotSize, angleDeg, flipX, flipZ]);
```

**改后：**
```typescript
}, [points, carX, carZ, width, height, dotColor, dotSize, angleDeg, flipX, flipZ]);
```

#### 4. 删除 console.log（约 544-546 行，可选）

调试日志中有 `carPos` 字段，可以改为输出 `carX`/`carZ` 或直接删除。

## acc-coach 端的对应变更

`acc-coach` 端的 `MapWidget` 组件已简化：移除了 `rotationDeg`/`flipX`/`flipZ` props，红点直接按传入的 `carX`/`carZ` 坐标绘制，不做任何额外变换。调用方负责确保 `carX`/`carZ` 和 `trackPoints` 在同一个坐标系内。
