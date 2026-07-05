# Delta Time to Session Best Lap — 重设计方案 v2（合并定稿）

> **状态**: 评审通过 / 可进入实施
> **日期**: 2026-07-05
> **范围**: `src/compute/items.rs` · `src/compute/registry.rs` · `src/compute/context.rs` · `src/trackmap.rs` · `src/dashboard/service.rs`
> **取代**:
> - `2026-07-03-delta-time-session-best-redesign.md`（v1 原设计）
> - `2026-07-04-delta-time-review-feedback.md`（评审反馈）
>
> **合并说明**: 本文档由 v1 设计 + 评审反馈 + 审计裁定三方合并而成。评审反馈中多数建议已采纳；§5 R-int 的"公式有 bug"定性经审计裁定为误判，已更正为"高采样率下收益有限，先用线性 `(s,t)`"。两处 R-int 实现陷阱（`v0→0` 奇异、`v` 异号 NaN）作为启用约束补入。根因主次重排：position 域量化为头号误差源。
> **相关文档**: [calculated-item.md](../api/calculated-item.md) · [sector-calculated-items.md](../reference/sector-calculated-items.md) · [computed-telemetry-logic.md](../reference/computed-telemetry-logic.md)

---

## 1. 背景与问题

当前内置两个 calculated item，均实现 `RealtimeComputeItem`，计算"当前圈相对本 Session 最佳圈的时间差"：

| Item | 代码位置 | 实现 |
|---|---|---|
| `delta_time_to_session_best_lap` | `src/compute/items.rs:316` | 离散扫描参考圈，找首个 `pos` 越过的采样点直接相减 |
| `delta_time_to_session_best_lap_interpolated` | `src/compute/items.rs:385` | 参考圈整理成单调 `(pos, time)`，二分 + 线性插值 |

### 1.1 根因诊断（按影响排序，已重排）

> **主次澄清**：原 v1 与反馈都把"恒速插值"列为 interpolated 版主要根因之一，但数量级上 **position 域量化才是头号误差源**。换匹配域是数量级提升；段内插值模型（线性 / R-int / PCHIP）是二阶小量。本方案的主线是"换匹配域"，段内插值模型次之。

1. **【头号】匹配域分辨率不足**：`normalized_car_position` 分辨率 0.001 → 5km 赛道约 5m/格。误差预算：200km/h≈55m/s，10ms≈0.55m——**仅 position 量化一项就超预算 10 倍**。高速段 position 域物理上达不到分析级精度。子米级弧长 `s` 必须自行派生（速度积分 / 中线弧长投影）。**此约束直接排除"在 position 域改良插值"的路线。**
2. **【二阶】插值模型**：`t = t0 + ratio*(t1−t0)`，`ratio=(p−p0)/(p1−p0)`，等价于假设两采样点间**速度恒定**（`dt/dp=const`）。弯道速度变化剧烈，真实 `t(p)=∫dp/v(p)` 是弯曲的，线性插值在每个弯道产生系统性 S 曲线偏差。**在 position 域内此偏差与 #1 同向叠加；迁到 s 域后段内仅剩此二阶项。**
3. **【整圈偏置】S/F 对齐**：圈起点用 `position` 跨 0.8→0.2 启发式，跨线样本不确定 ±1 帧，引入整圈恒定偏置。
4. ACC `SessionSample` **无"圈距离(米)"字段**（仅有 `normalized_car_position`/`completed_laps`/`current_sector_index`），子米级弧长 `s` 必须自行派生。

本方案先用验证设施量化上述根因，再分阶段落地。

---

## 2. 目标与非目标

### 目标
- **精度优先（分析级）**：局部（含弯道）可信，目标 ±10ms 量级。
- **实时性**：作为 dashboard 数据，`compute()` 每帧调用（帧驱动 ~50–100Hz，见 `process_frame` `src/dashboard/service.rs:527`），单次 ≪1ms。
- **可量化验证**：有离线回放验证设施，能对任意方法输出 δ-vs-s 误差曲线。

### 非目标
- 不替换 ACC 内置 delta（仅做自研参考）。
- 不在本期实现 `delta_time_to_life_best_lap` 的新算法（同架构可后续复用）。
- 不引入 GPU/跨进程并行（单 delta 无法并行；资源应投在预计算）。

### 约束确认（来自需求澄清）
- delta 语义：**保持"同位置时间差" `δ(s)=t_cur(s)−t_best(s)`**（方案 A/B）；方案 C 改变语义，仅作可选极速项。
- 参考圈更新频率：**很少（仅新最佳圈触发）** → 预计算可重。
- 赛道中线建表：**可接受**（有 `track_name` 识别）。

---

## 3. 可用信号（已核实）

| 信号 | 字段 | 说明 |
|---|---|---|
| 赛道进度 | `session.normalized_car_position` (f32, 0..1) | ACC 归一化位置，分辨率受限 |
| 圈号 | `session.completed_laps` (i32) | S/F 离散判据 |
| Sector | `session.current_sector_index` (i32) | 漂移重同步可用 |
| 圈内时刻 | `timing.i_current_time` (i32 ms) | 即 `t_cur`，也是 t 域下标 |
| 速度向量 | `motion.velocity[3]` (f32, m/s 世界系) | 车速 `=hypot(v[0],v[2])` |
| 玩家世界坐标 | `other_cars.car_coordinates[player_car_id*3 .. +2]` (f32) | XZ 平面，Y=高度忽略 |
| 玩家车索引 | `other_cars.player_car_id` (i32) | **关键**：见 §4.1 |
| 时序 | `sample_tick` / `timestamp_ns` (u64) | 帧对齐 |

> 参考圈 `&[TelemetryFrame]` 同样携带 `motion` + `other_cars`，故参考圈每帧也有速度与坐标。

---

## 4. 共用基建（所有方案均依赖，须先落地）

### 4.1 玩家 XY 取法修正（前置，优先级最高）

**现状问题**：`CarCoordX`/`CarCoordZ`（`src/compute/items.rs:504` / `:524`）取 `other_cars.car_coordinates[0]` / `[2]`，**仅当 `player_car_id==0` 时正确**。`OtherCarsSample.car_coordinates` 是 60 车 × 3 的扁平数组（`src/types.rs:935`），玩家车须用 `player_car_id` 索引。这是明确的 latent bug。

**正确取法**（已存在于 `src/trackmap.rs:82-90`，可抽公共）：
```
player_idx = other_cars.player_car_id as usize;
base = player_idx * 3;
x = car_coordinates[base];
z = car_coordinates[base + 2];
```

**落地**：新增公共函数 `player_xz(other: &OtherCarsSample) -> Option<(f32, f32)>`（放 `src/compute/util.rs`），带越界保护：

```rust
pub fn player_xz(other: &OtherCarsSample) -> Option<(f32, f32)> {
    let idx = other.player_car_id as usize;
    let base = idx * 3;
    if base + 2 < other.car_coordinates.len() {
        Some((other.car_coordinates[base], other.car_coordinates[base + 2]))
    } else {
        None
    }
}
```

- 修正 `CarCoordX`/`CarCoordZ` 复用之，并补单元测试。
- 新 delta item 内部直接调用，不依赖 `car_x`/`car_z`。

### 4.2 S/F 亚帧对齐（F-sub，必做）

**目的**：消除整圈恒定偏置，±10ms 前提。

**前置验证（P0 必做）**：先用 G-replay（§4.3）读真实 `.acctlm2`，确认同一圈 S/F 处 `i_current_time` 与 `normalized_car_position` 的跳变规律——**`i_current_time` 是否每圈从 0 起算且物理过 S/F 线时归零**。若是，`t_cur` 直接用 `i_current_time`；若否，F-sub 需补偿整圈偏置。此结论影响 delta 语义本身，须先实测。

**算法**：跨线两帧 `(t0,p0,v0)`、`(t1,p1,v1)`（`p1` 突降），在两帧间求精确过线时刻 `t_cross` 与对应 `s_cross`：
- position 域线性（baseline）：`t_cross = t0 + (t1−t0) * (p0 / (p0 + (1−p1)))`；
- 速度感知（更准，可选）：以 `v` 线性积分 `Δs` 反推过线比例。
- 当前圈 `s` 积分从 `t_cross` 起算，`t_cur = i_current_time − t_cross`；参考圈表 `t_best(s)` 的 `s=0` 对齐到 `t_cross` 处。

**判据组合**（取代单一 position 启发式）：
- 主判据：`completed_laps` 跳变（最干净）；
- 辅判据：`i_current_time` 归零、`normalized_car_position` 跨 0.8→0.2；
- 三者中任一触发即候选，取一致的那次。

**落地**：参考圈预算与实时 `s` 估计器初始化共用一个 `sf_align.rs` 工具模块。

### 4.3 验证设施（G-replay，精度优先须先搭，是所有技术决策的前提）

**离线回放 harness**（落 `tests/compute_tests.rs` 新 `delta_accuracy` 模块）：
1. 取一 `.acctlm2` 文件的两圈高质量 flying lap A、B；
2. 各用同一方法建精确 `(s,t)` 表；
3. 以 A 为参考算 `δ_AB(s)`，以 B 为参考算 `δ_BA(s)`，自洽检验 `δ_AB(s) ≡ −δ_BA(s)`；
4. 与现有 `interpolated` 叠加，画 δ-vs-s 分歧段定位偏差源。

**harness 须回答的三个问题**（决策前提）：
1. 现有 `interpolated` 的误差形态？（弯道是否有 S 形系统偏差？直道是否稳定？）
2. 方案 A 相比 `interpolated`，弯道 δ 波动是否显著降低？
3. 自洽检验 `δ_AB(s) = −δ_BA(s)` 是否成立？

**段内插值模型对照**：harness 应能同时跑线性 `(s,t)` 与 R-int（§5）两版段内插值，用数据决定哪个上线——这正是"用数据说话"。

**逐帧 dump**：`(sample_tick, s_cur, t_cur, matched_seg, s_best, t_best, ref_v, δ)` 落 CSV，供离线绘图。

**运行时计量**：`process_frame` 已记 `last_compute_duration_ns`/`max_compute_duration_ns`（`src/dashboard/service.rs:640-648`），直接复用验证实时预算。

---

## 5. 参考圈重建：段内插值模型（P1 默认线性，R-int 可选升级）

> **审计裁定**：v1 把 R-int 列为"共用基建必先落地"；反馈 §3.4 称"R-int 公式有 bug 不能用"。经审计，**两者定性都偏了**：
> - R-int 公式数学正确（假设 `v(s)` 线性时 `t(s)=t0+∫ds/v(s)` 的闭式解），`t(s1)≠t1` 是模型性质（用 `v0,v1` 预测 `t1`），非 bug；逐段重置 `t0` 为实测值时**不累积系统性偏差**。
> - 但 ACC 50–100Hz、帧间 ~1–2m，R-int 相对线性 `(s,t)` 的收益是 `O((Δs)²)` 二阶小量，远小于已消除的 position 域 5m 量化误差。**复杂度不值**。
> - 故 P1 默认线性 `(s,t)`；若 G-replay 证明段内插值仍是主要误差源，再上 PCHIP 或 R-int。

### 5.1 P1 默认：线性 `(s,t)` 插值

```
t(s) = t0 + (t1 − t0) * (s − s0) / (s1 − s0)
```
- 隐含恒速假设 `v=(s1−s0)/(t1−t0)`；段内 ~1–2m 误差 `O((Δs)²)`，可忽略。
- 过所有实测采样点 `(s_i, t_i)`，无总时间失真。
- 退化（`s1≈s0`）退化为 `t0`。

### 5.2 可选升级 A：monotonic PCHIP（Fritsch-Carlson）

若线性不够平滑、出现 wiggle：用 PCHIP 插值 `(s,t)`，**保物理单调**（`t` 对 `s` 不倒流），过所有采样点。普通三次样条/Akima 会过冲破坏单调，禁用。

### 5.3 可选升级 B：R-int 速度感知插值（含实现约束）

**核心公式**（假设 `v(s)` 在段内线性）：
```
t(s) = t0 + (s1−s0)/(v1−v0) * ln( (v0 + (v1−v0)*r) / v0 )
其中 r = (s − s0) / (s1 − s0)
```
退化（`|v1−v0|` 极小）：用梯形 `t(s) ≈ t0 + 2*(s−s0)/(v0+v1)`。

> **注意**：此公式用 `(s0,t0,v0,v1)` 预测 `t1`，**不强制过实测 `t1`**——这是模型性质，非 bug。预重采样逐段做、每段起点 `t0` 取实测值时，不累积系统性偏差。是否启用由 G-replay 数据决定。

**启用 R-int 时的实现约束（必做，v1 与反馈均未覆盖）**：
1. **`v0→0` 奇异**：`ln((v0+...)/v0)→∞`。退化分支只覆盖 `|v1−v0|` 小，**未覆盖 `v0≈0`**。低速弯 / pit 出口会爆。需加 `v0 < ε` 时退化为线性或截断。
2. **`v0`、`v1` 异号**（倒车 / spin）→ `ln` 负数 → NaN。需符号守卫，异号段退化为线性。

### 5.4 预重采样（R-resample）

用选定段内插值（线性 / PCHIP / R-int）把参考圈重采样到密集均匀 `s` 网格，存为 `Vec<f32>` 量化表。实时只剩二分或直接下标。

**bin 数**：`N=65536` → 5km 约 0.076m/格，内存 512KB/圈，可忽略。精度富裕，**P1 直接用 65536**（反馈建议，原 v1 在 16384/65536 间犹豫）。

---

## 6. 方案 A — s-R 参考折线投影（推荐先行，P1）

**思路**：参考圈自身的 XZ 轨迹即带弧长的折线——它就是"中线"。实时把当前 XY 投影到参考折线得 `s`，查 `t_best[s]`，`δ = i_current_time − t_best`。**无需每赛道建表**。

### 6.1 数据结构（参考圈更新时预算，内存）

```rust
struct RefPoly {
    pts: Vec<(x: f32, z: f32, t_ms: f32, v: f32)>,  // 参考圈每帧
    cumlen: Vec<f32>,                                 // cumlen[i] = 到 pts[i] 的累计弧长
    total_len: f32,
    // 量化查表（O(1)）
    t_best_bins: Vec<f32>,                            // 长度 N=65536，t_best[bin] = 参考圈在 bin*step 的时刻
    bin_step: f32,
    signature: (usize, usize, u64, u64),              // (ptr,len,first_tick,last_tick) 沿用 items.rs:404
}
```

### 6.2 构建（新最佳圈触发，可重）

1. 对参考圈每帧取 `(x,z, i_current_time, speed=hypot(v[0],v[2]))`；
2. 去静止/低速点（`v<ε`），避免 pit / grid 阶段噪声；1–2 帧去抖平滑；
3. S/F 亚帧对齐（§4.2）确定 `s=0`，**不要简单把两段折线拼起来**；
4. `cumlen` 由相邻点欧氏距离累加；保证 `t` 随 `cumlen` 单调（取累计 max 修正回退）；
5. 用 §5 选定的段内插值把 `(cumlen, t, v)` 重采样到均匀 `s` 网格 → `t_best_bins`。

刷新检测：`signature` 不变则跳过（同 `refresh_reference_points` 模式 `src/compute/items.rs:404`）。

### 6.3 实时 compute()（每帧，O(1)）

```
1. cur_xy = player_xz(other_cars)                        // §4.1
2. seg = project_local(cur_xy, RefPoly, last_seg, k=4)   // 时序连贯：[last_seg−4, last_seg+4] 找最近段
3. s_cur = cumlen[seg] + project_ratio * seg_len(seg)    // 投影到该段的弧长
4. 跨 S/F wrap（s 超出 total_len 或回退 → 圈界处理）
5. t_best = t_best_bins[ (s_cur / bin_step) as usize ]   // O(1)；可选相邻 bin 线性微调
6. δ = i_current_time − t_best
7. O-clamp ±5s + 短窗 median
```

### 6.4 `project_local` 实现要点（反馈补充）

- 扫 `[last_seg−4, last_seg+4]` 约 9 段，**跨 S/F 时窗口要 wrap**；
- 高速 / 低帧率下 9 段可能不够，建议按"最大帧间弧长"动态覆盖，如 100–200ms；
- 用 **2D 点到线段距离** 投影，**不是点到点**。

### 6.5 参考折线 bias（反馈补充，方案 A 的理论上限）

方案 A 的 `s` 不是赛道几何中线，而是**参考圈那一条走线的弧长**。当前圈与参考圈走线不同（入弯点、出弯点、spin 后回赛道）时：
- 同一弯道，不同走线的 `s` 不对应同一物理位置；
- 投影距离大时，最近点可能跳到其他段，delta 会跳。

**置信度门控（明确）**：
- 投影距离 > 阈值（如 3–5m）→ 输出 `NoValidData` 或 hold 上一帧；
- 局部搜索窗口失效时 fallback 到全局最近点。

阈值需用 G-replay 实测标定（见 §10 决策 3）。

### 6.6 代码落点

- 新 struct `DeltaTimeToSessionBestRefPoly`（`src/compute/items.rs`），impl `RealtimeComputeItem`。
- 字段：`RefPoly` + `last_seg: usize` + `last_lap: i32` + 输出 median 短窗。
- 注册：加入 `all_builtin_calculated_items()`（`src/compute/items.rs:578`）与默认 `ComputeRegistry` 初始化。
- 参考圈来源：`ctx.reference_lap`（已由 `ComputeRegistry::replace_reference` 注入 session-best，`src/compute/registry.rs:249`）。
- 投影/取坐标公共函数放 `src/compute/util.rs`，trackmap.rs 复用。

### 6.7 精度 / 速度 / 风险

- **精度**：`s` 误差 ≈ 参考圈采样间距（好采样亚米）+ 投影误差（亚米）；线性段内插值的二阶误差可忽略；F-sub 消除恒定偏置。可达 ±10ms 量级。**理论上限受参考折线 bias 限制（§6.5）**。
- **速度**：投影扫 ~9 段 + 一次数组读 ≈ 数百 ns。
- **风险**：走线差异大时局部搜索错段 → §6.5 置信度门控。
- **优点**：开箱即用、无赛道表；参考圈换时自动重建。

---

## 7. 方案 B — s-C 赛道中线 + 量化表（长期最稳，P3 后置）

**思路**：每赛道建一次独立中线折线（弧长参数化），实时投影当前 XY 到中线得 `s`；参考圈预算 `t_best[s]` 量化表。`s` 域独立于任何单圈走线，最稳。

### 7.1 数据结构

```rust
struct Centerline {
    points: Vec<(x: f32, z: f32)>,   // 等弧长重采样，如每 1m（或 0.3m）
    cumlen: Vec<f32>,
    total_len: f32,
    track_id: String,                // metadata.track_name
}
// 持久化：trackmaps/<track_id>.bin
```

参考圈 `t_best` 量化表同方案 A 的 `t_best_bins`，但 `s` 域是赛道中线弧长（参考圈帧投影到中线得 `s_ref`，再 §5 重采样）。

### 7.2 构建

**赛道中线（每赛道一次）**：
- 首次见到某 `track_name`：取第一圈有效 flying lap 的 XZ 轨迹；
- 去重、等弧长重采样到 1m（或 0.3m 加密）；
- 算 `cumlen`；落盘 `trackmaps/<track_id>.bin` 复用。
- 复用 `src/trackmap.rs` 的 `extract_track_coordinates`（:48）+ `detect_lap_crossings`（:216）。

**参考圈表（换最佳圈时）**：
- 参考圈每帧投影到中线得 `s_ref` + `t=i_current_time` + `v`；
- §5 重采样到均匀 `s` 网格 → `t_best_bins`；
- F-sub 对齐 `s=0` 到起跑线（F-center）。

### 7.3 实时 compute()

与方案 A 几乎相同，区别：投影目标是 `Centerline` 而非 `RefPoly`；`s` 域是赛道弧长。其余步骤（查表、δ、clamp、median）一致。

### 7.4 代码落点

- 新 struct `DeltaTimeToSessionBestCenterline`（`src/compute/items.rs`）。
- 中线构建/存取：新模块 `src/trackmap/centerline.rs`（或扩 `src/trackmap.rs`），按 `track_id` 缓存 + 磁盘复用。
- `track_id` 经 §10 决策 1 注入 `ComputeContext`。

### 7.5 精度 / 速度 / 风险

- **精度**：`s` 误差 ≈ 中线分辨率（1m → 可加密 0.3m）；段内插值同方案 A。长期最稳，独立于单圈走线噪声。
- **速度**：同方案 A（O(1)）。
- **风险/成本**：需每赛道一次性建表 + 磁盘管理；中线 `s=0` 须与 S/F 对齐（F-center）。
- **何时上**：方案 A 验证后若走线差异扰动大或精度仍不足，升级到 B。

---

## 8. 方案 C — t 域反向表 + 速度门控混合（可选极速，语义改变，不作主线）

> **反馈与审计一致**：改变 delta 语义，不作 dashboard 主 delta。仅在"语义可换且要极致响应"时作为并行 fast 项。

**思路**：参考圈存"圈内时刻 t → 参考圈位置 `s_best(t)`"，`i_current_time` 即实时下标，零搜索。δ 语义变为"距离差/速度"。

### 8.1 数据结构

```rust
struct InvTable {
    s_at_t: Vec<f32>,   // 按 1ms 一格，一圈 ~120s → 120000 格 ~1MB；或按原始 (t,s) 二分
}
// 同时保留方案 A/B 的 t_best_bins（s 域）用于低速段切换
```

### 8.2 构建

参考圈 `(i_current_time, s_ref)`（`s_ref` 来自中线或参考折线投影），§5 插值到均匀 `t` 网格 → `s_at_t`。

### 8.3 实时 compute()（O(1)）

```
v_cur = hypot(velocity[0], velocity[2])
s_best = s_at_t[i_current_time]          // 零搜索
s_cur  = (速度积分 或 快速投影)           // 需一个轻量 s 估计器
δ_t = (s_cur − s_best) / v_cur           // 高速段
δ_s = i_current_time − t_best[s_cur]     // 低速段（s 域，方案 A/B）
w = v_cur / (v_cur + v_thresh)           // v_thresh ~ 10 m/s
δ = w * δ_t + (1−w) * δ_s
```

### 8.4 精度 / 速度 / 风险

- **高速段**：零搜索、零 s 匹配误差，最响应。
- **低速弯**：t 域病态（`v→0` 除法爆炸）→ 切 s 域。
- **速度**：两次 O(1) 查表 + 一次混合，最快档。
- **代价/风险**：**语义改变**——高速段报"距离差折算时间"，与"同位置时间差"在弯道进出口可能差几十 ms，overlay 上有跳变风险（需平滑过渡）。
- **落地**：若做，作为单独 item `delta_time_to_session_best_lap_fast`，明确标注语义不同。

---

## 9. 输出鲁棒性（所有方案共用，保持轻量）

- **O-clamp**：`|δ|` 限幅 ±5s，杀异常。
- **O-median**：短窗 **3 帧**中值 + 滞回（反馈建议 3 而非 5，5 帧延迟偏多；约 30–60ms 可接受），杀单帧尖刺不引入滞后。
- **O-conf / O-hold**：先做简单版——投影距离大和低速时输出 `NoValidData` / 短暂冻结 δ，**不必做复杂状态机**。低速弯 `v` 小 → 同 `s` 误差放大为更大时间误差，附置信度标记（可由 dashboard 侧弱化显示）。

---

## 10. 已定决策（原开放问题，评审后落实）

| # | 问题 | 决策 |
|---|---|---|
| 1 | `track_id` 获取 | **选 (b) 扩 `ComputeContext` 增加 `track_name`/`track_id` 字段**。`TelemetryFrame` 无赛道名但 metadata 有；`DashboardService` 初始化或收到 metadata 后注入。成本最低，方案 A 用不到也先加。 |
| 2 | `i_current_time` 起点一致性 | **P0 用 G-replay 实测确认**（§4.2 前置）。若不每圈从 0 起算 / 不与 S/F 物理过线对齐，F-sub 需补偿。 |
| 3 | 走线差异阈值 | **用 G-replay 实测标定**方案 A 投影置信度门控阈值。 |
| 4 | 是否保留现有两个 item | **不替换，新增 `delta_time_to_session_best_lap_refpoly` 并行**，验证达标后再废弃 `..._interpolated`。 |
| 5 | bin 粒度 | **65536**（5km≈0.076m/格，512KB/圈，精度富裕）。 |
| 6 | 段内插值模型 | **P1 默认线性 `(s,t)`**；G-replay 证明段内插值仍是主要误差源 → 上 PCHIP 或 R-int（含 §5.3 实现约束）。 |
| 7 | delta 语义 | **保持"同位置时间差" `δ(s)=t_cur(s)−t_best(s)`**（方案 A/B 为主）。 |
| 8 | §4.1 XY 修正顺手修 `car_x`/`car_z` | **同意**。 |

---

## 11. 分阶段落地路线（评审后定稿）

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **P0 基建** | 1. §4.1 玩家 XY 修正（含 `CarCoordX`/`CarCoordZ` + 单测）<br>2. §10 决策 1：扩 `ComputeContext` 加 `track_name`/`track_id`<br>3. §4.3 G-replay harness（读 `.acctlm2`、选两圈、输出 δ-vs-s CSV、自洽检验）<br>4. §4.2 前置：实测 `i_current_time` S/F 跳变规律<br>5. 用现有 `interpolated` 跑基准误差曲线 + 对照线性/R-int 段内插值 | 有现有方法的误差形态图；`i_current_time` 语义结论 |
| **P1 方案 A** | `DeltaTimeToSessionBestRefPoly`：参考折线投影 + 均匀 s 网格(65536) + **线性 `(s,t)` 插值** + 简单 F-sub + O-clamp/median(3) + 置信度门控 | G-replay 自洽 `δ_AB=−δ_BA`；弯道 S 形偏差显著减小；`compute()` <1ms |
| **P2 评估** | 对比 A 与现有 `interpolated` 的 δ-vs-s；若 A 精度达标 → 并行新增 item；若走线差异大 → 升级 B；若段内插值是主因 → 升级 PCHIP/R-int | 局部 ±10ms 量级 |
| **P3 方案 B（条件）** | 赛道中线 + 持久化 `trackmaps/<track_id>.bin` | 跨圈/跨走线稳定 |
| **P4 方案 C（可选）** | 若需极致响应且接受语义切换 → 并行 `delta_time_to_session_best_lap_fast` | 语义一致性评审通过 |

每阶段可独立评审与合并；P1 不依赖 P3 的赛道表。

### 11.1 起步两个 PR（反馈建议，可直接执行）

1. **PR 1: Bugfix** — 修正 `CarCoordX`/`CarCoordZ` 的 `player_car_id` 索引，抽公共 `player_xz`，补单测。
2. **PR 2: Delta accuracy harness** — 在 `tests/compute_tests.rs` 新增离线回放测试，能读取 `.acctlm2`、取两圈、输出 δ-vs-s CSV，跑通自洽检验 `δ_AB = −δ_BA`。

等 harness 跑出现有 `interpolated` 的 baseline 数据后，再决定方案 A 的实现细节。

---

## 附录 A — 探索的方案空间与取舍

> 保留 v1 脑暴过程：考虑过哪些方向、为何取舍、为何收敛到 §6–§8 的 A/B/C 三方案。避免评审时重复争论已讨论过的方向，并显露决策依据。

### A.1 误差预算（硬约束，决定方案空间）

200 km/h ≈ 55 m/s，**10 ms ≈ 0.55 m**。而 `normalized_car_position` 分辨率按 0.001 计，5 km 赛道 ≈ 5 m/格——**仅 position 量化这一项就超预算 10 倍**。

推论：**任何只依赖 `normalized_car_position` 的方案在高速段物理上达不到分析级精度**。子米级 `s` 必须自行派生（速度积分 / 中线弧长投影）。这条约束直接排除了"继续在 position 域改良插值"的路线，把方案空间收窄到"换匹配域"。

### A.2 被考虑过的方向（按层分类）

| 层 | 候选 | 备注 |
|---|---|---|
| **匹配域 s** | s-P(position) · s-V(速度积分) · s-C(中线弧长) · s-H(粗门+细积分) · s-K(Kalman) · s-R(参考折线投影) | s-P 单用精度不够；其余可行 |
| **参考重建** | R-lin(线性) · R-int(速度感知) · R-pchip(单调三次) · R-resample(预重采样) · R-bspline(B 样条) · 普通三次样条 | 普通三次破坏单调，弃；**R-lin 为 P1 默认**，R-int/R-pchip 为可选升级 |
| **实时 s 估计** | L-proj(投影) · L-int(积分) · L-kalman · L-gate(position 异常门) | 均可叠加 |
| **S/F 对齐** | F-wrap · F-lap · F-time · F-sub(亚帧) · F-center(中线零点) | F-sub 必备 |
| **输出鲁棒** | O-clamp · O-median · O-conf · O-hold | 全部纳入共用（P1 取轻量版） |
| **备选语义** | S-point(同位置时间差) · S-arrival(前向到达) · S-mini(mini-sector) · S-virtual(虚拟最佳) · S-phase(弯道相位) · S-t-domain(距离差/速度) · DTW · 互相关 | 多数弃，见 A.4 |

### A.3 精度与速度同向（关键洞察）

实时热路径是 `process_frame`（`src/dashboard/service.rs:527`）每帧同步调用 `compute()`，单线程顺序。单 delta 无法并行，跨 item 并行对单 delta 无意义。

但所有精度赢家（R-lin / R-int / R-resample / 量化表 / 中线投影）的**重活都在参考圈更新时一次性预算**（罕见、可重），实时 `compute()` 只剩一次投影 + 一次数组读 = O(1)，比现状的 O(n) 扫描还快。

**结论：精度与速度不冲突，反而同向。** "机器资源多"应投在预计算密度 / 空间索引 / 多估计器冗余，而非并行。这让"上 R-resample + 量化表"没有实时性代价。

### A.4 被排除/降级的方向及理由

- **普通三次样条 / Akima**：对 `(s,t)` 拟合会产生过冲 wiggle，破坏 `t` 对 `s` 的物理单调性 → 时间倒流。改用 **R-pchip(Fritsch-Carlson)** 保单调，或 **R-int**（基于物理 `v`，不引入 wiggle）。P1 选 R-lin 起步，R-int / R-pchip 为可选升级。
- **DTW 轨迹对齐**：数学上"最优"非线性时间对齐，但非实时友好（O(n²)），且语义模糊。弃。
- **互相关全局对齐**：对速度/距离曲线做互相关只能给一个"整圈 delta"标量，无法逐点。仅适合 overlay 的平滑总差，不适合分析级逐点 δ。降级为"未来可选的 overlay 平滑项"。
- **S-mini(mini-sector 累计)**：极稳但精度低（分段粒度），广播级而非分析级。与"精度优先"目标冲突，降级为备选。
- **S-virtual(虚拟最佳圈)**：拼接各 mini-sector 历史最佳段做参考，去单圈噪声；但参考非真实圈，语义偏离"本 session 最佳圈"。降级为"参考圈降噪的可选增强"。
- **S-phase(弯道相位对齐)**：按刹车点/入弯点对齐而非 s，无稳定判据。弃。
- **S-arrival(前向到达时间差)**：与 S-point 在 s=0 附近有别、更平滑，但语义不直观且需前向预测。暂不取，记为备选。
- **IMU 双积分类距离**：漂移过大，弃。
- **ACC 内置 delta / `gap_*` 字段**：`SessionSample.gap_ahead_or_tail_value`/`gap_behind` 是车间距，非圈 delta；ACC 内置 delta 已实测不准。均弃。

### A.5 t 域反向表的诱惑与代价（方案 C 的取舍）

`t 域反向表` `s_best(t)` 用 `i_current_time` 直接下标，**零搜索、零 s 匹配误差**，是所有方案里实时最快、高速段最响应的。但它的 δ 语义是 `(s_cur−s_best)/v`（距离差折算时间），**与"同位置时间差"在弯道进出口会差几十 ms**，overlay 上有跳变风险。

取舍：**不作主 delta**（语义不一致会让分析困惑），仅在"语义可换且要极致响应"时作为并行 fast 项（§8）。主 delta 仍用 s 域（方案 A/B），保留 S-point 语义。

### A.6 为何收敛到 A/B/C

从 A.2 的候选层组合中，按"精度优先 + 实时无忧 + 工程量递增"筛出三条主线：

1. **方案 A (s-R)**：参考折线投影 + R-lin + 量化表。**免赛道建表、开箱即用**，精度上限相当，适合先行验证根因。隐含假设"参考圈走线≈当前走线"，加置信度门控可扛小差异。
2. **方案 B (s-C)**：赛道中线 + 量化表。`s` 域独立于任何单圈走线，**长期最稳**，但需每赛道一次性建表。作为 A 验证后的升级路径。
3. **方案 C (t 域)**：极速但语义改变，**仅作可选并行项**。

三者共享 §4 基建（玩家 XY 修正 / F-sub / G-replay）与 §5 段内插值。A→B 是"从轻到重、每步可量化验证"的递进，避免一上来背赛道建表负担；C 不阻塞主线。

其余候选（PCHIP/B 样条/Kalman/双估计器/共识轨迹等）作为各层内的**可选增强**保留，不单独成方案——例如 PCHIP 可在线性不够平滑时替换段内插值；Kalman 可在实时 s 抖动大时替换 L-proj；共识轨迹可在参考圈噪声大时作为参考源。这些都在 §6–§9 的对应层留有接口，无需预先定案。

