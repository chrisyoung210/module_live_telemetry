# 通道命名规范

> 适用范围：`acc-coach`（含前端）、`module_local_dashboard`、`module_dashboard_protocol`
>
> 参考方：`module_live_telemetry`（不受本规范约束）
>
> 状态：已定稿

---

## 一、两种命名约定

本系统存在两种通道命名约定，分属不同层面：

| 约定 | 格式 | 示例 | 所属模块 | 使用范围 |
|------|------|------|----------|----------|
| **规范格式** | `{类型}:{子系统}.{字段}` | `raw:controls.speed_kmh` | `module_live_telemetry` | 订阅、验证、原始数据标识 |
| **用户格式** | `camelCase` | `speedKmh` | `acc-coach`、`module_local_dashboard` | UI 展示、layout 字段绑定、frame 数据 key |

**铁律**：除了 `acc-coach/src/dashboard/alias.rs` 这一层外，任何模块不得同时感知两种格式。

---

## 二、用户格式生成规则

### 2.1 自动生成（覆盖 ~80% 字段）

从规范格式的 `{子系统}.{字段}` 部分自动推导：

```
输入:  {子系统}.{field_name_1_field_name_2}
步骤:
  1. 去掉子系统前缀（第一个 . 之前的部分）
  2. 剩余部分按下划线 split
  3. 首单词全小写，后续每个单词首字母大写，拼接
输出:  用户名
```

示例：

| 规范名 | 去掉前缀 | 驼峰转换 | 用户名 |
|--------|----------|----------|--------|
| `controls.speed_kmh` | `speed_kmh` | `speedKmh` | `speedKmh` |
| `controls.speed_mph` | `speed_mph` | `speedMph` | `speedMph` |
| `controls.gear` | `gear` | `gear` | `gear` |
| `car_state.abs_level` | `abs_level` | `absLevel` | `absLevel` |
| `session.is_valid_lap` | `is_valid_lap` | `isValidLap` | `isValidLap` |
| `tyres.brake_temp[0]` | `brake_temp[0]` | `brakeTemp[0]` | `brakeTemp[0]` |
| `powertrain.water_temp` | `water_temp` | `waterTemp` | `waterTemp` |

### 2.2 显式覆盖（~15 个字段）

当自动生成结果语义不正确时，使用覆盖表：

| 规范名 | 自动结果 | 覆盖为 | 原因 |
|--------|----------|--------|------|
| `controls.gas` | `gas` | `throttlePct` | 用户理解："油门百分比" |
| `controls.brake` | `brake` | `brakePct` | 用户理解："刹车百分比"，加单位后缀 |
| `controls.clutch` | `clutch` | `clutchPct` | 用户理解："离合百分比"，加单位后缀 |
| `controls.rpms` | `rpms` | `rpm` | 去掉末尾 s，统一单数 |
| `controls.steer_angle` | `steerAngle` | `steerRawAngle` | 区分原始角度和计算后的 steeringDeg |
| `timing.i_current_time` | `iCurrentTime` | `currentLapTimeMs` | 语义：当前圈用时 |
| `timing.i_last_time` | `iLastTime` | `lastLapTimeMs` | 语义：上一圈用时 |
| `timing.i_best_time` | `iBestTime` | `bestLapTimeMs` | 语义：最佳圈用时 |
| `timing.i_delta_lap_time` | `iDeltaLapTime` | `bestLapDeltaTimeMs` | 语义：与最佳圈时间差 |
| `timing.i_estimated_lap_time` | `iEstimatedLapTime` | `predictedLapTimeByBest` | 语义：预估圈时 |
| `timing.last_sector_time` | `lastSectorTime` | `lastSectorTime` | 自动正确，不覆盖 |
| `timing.i_split` | `iSplit` | `iSplit` | 自动正确，不覆盖 |

> **维护约定**：`raw_catalog` 新增字段时，若不在此覆盖表中，自动使用规则生成。若语义不对，在此表追加覆盖条目。

### 2.3 同义词（多对一）

部分用户名指向同一个规范 key，`user_to_canonical` 支持多个输入名：

| 主名称 | 同义词 | 规范名 |
|--------|--------|--------|
| `bestLapTimeMs` | `sessionBestLapTimeMs` | `raw:timing.i_best_time` |
| `bestLapDeltaTimeMs` | `sessionLapDeltaTimeMs` | `raw:timing.i_delta_lap_time` |
| `predictedLapTimeByBest` | `predictedLapTimeBySession` | `raw:timing.i_estimated_lap_time` |
| `steerRawAngle` | `steeringDeg` | `raw:controls.steer_angle` |

构建时在 `user_to_canonical` 表中，主名称与其每个同义词都作为独立 key 登记指向同一规范名；查询时 `user_to_canonical` 对主名称和同义词均返回该规范名，`canonical_to_user` 只登记主名称（因此只返回主名称）。详见 `channel-alias-layer.md` 第二节构建伪代码中的"注册同义词"步骤。

---

## 三、各模块应使用的格式

```
module_live_telemetry
    ↓ 只使用规范格式 (raw:controls.speed_kmh)
    ↓
[alias.rs]     ← 唯一翻译点
    ↓
acc-coach 内部   ← 只使用用户格式 (speedKmh)
    ↓
frame.values    ← key 是用户格式
layout.controls ← telemetryField 是用户格式
channel.id      ← 用户格式
    ↓
module_local_dashboard
    ↓ 只使用用户格式，直接 frame.values["speedKmh"] 命中
```

---

## 四、关于 `speed_mph` 和 `speed_kmh`

这是 ACC 在不同单位制下的两个不同遥测字段，在 `raw_catalog` 中都有定义：
- `controls.speed_kmh` → 用户名 `speedKmh`
- `controls.speed_mph` → 用户名 `speedMph`（不在覆盖表中，规则自动生成）

overlay 的 `telemetryField` 应由用户/编辑器选择具体的通道（`speedKmh` 或 `speedMph`），`module_local_dashboard` 不应再通过别名表做 `speedKmh → speed_mph` 的自动回退。

---

## 五、关于 `timing.i_current_time`（点号） vs `timing_i_current_time`（下划线）

规范格式使用点号作为子系统分隔符：`timing.i_current_time`。若某些 frame 推送了 `timing_i_current_time`（下划线），这是数据产出端的 bug，应在源头修复。别名层不对下划线格式做兼容。

---

## 六、修订记录

| 日期 | 修订内容 |
|------|----------|
| 2026-06-27 | 初版，定义规则生成 + 覆盖表 + 同义词体系 |
