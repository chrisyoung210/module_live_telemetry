# Delta Time to Session Best Lap — 重设计方案

> **状态**: 评审中 (Draft)
> **日期**: 2026-07-03
> **范围**: `src/compute/items.rs` · `src/compute/registry.rs` · `src/trackmap.rs` · `src/dashboard/service.rs`
> **相关文档**: [calculated-item.md](../api/calculated-item.md) · [sector-calculated-items.md](../reference/sector-calculated-items.md) · [computed-telemetry-logic.md](../reference/computed-telemetry-logic.md)

---

## 1. 背景与问题

当前内置两个 calculated item，均实现 `RealtimeComputeItem`，计算"当前圈相对本 Session 最佳圈的时间差"：

| Item | 代码位置 | 实现 |
|---|---|---|
| `delta_time_to_session_best_lap` | `src/compute/items.rs:316` | 离散扫描参考圈，找首个 `pos` 越过的采样点直接相减 |
| `delta_time_to_session_best_lap_interpolated` | `src/compute/items.rs:385` | 参考圈整理成单调 `(pos, time)`，二分 + 线性插值 |

**问题**：
- 离散版每个采样点有量化误差（最高 ~一帧间隔），ACC 内置 delta 同样不准，故未使用；
- 插值版仍有偏差，根因（诊断假设）：
  1. **插值模型**：`t = t0 + ratio*(t1−t0)`，`ratio=(p−p0)/(p1−p0)`，等价于假设两采样点间**速度恒定**（`dt/dp=const`）。弯道速度变化剧烈，真实 `t(p)=∫dp/v(p)` 是弯曲的，线性插值在每个弯道产生系统性 S 曲线偏差。
  2. **匹配域**：`normalized_car_position` 分辨率有限（0.001 → 5km 赛道约 5m/格），高速段 (200km/h≈55m/s) 下 10ms≈0.55m，position 域物理上达不到分析级精度。
  3. **S/F 对齐**：圈起点用 `position` 跨 0.8→0.2 启发式，跨线样本不确定 ±1 帧，引入整圈恒定偏置。
  4. ACC `SessionSample` **无"圈距离(米)"字段**（仅有 `normalized_car_position`/`completed_laps`/`current_sector_index`），子米级弧长 `s` 必须自行派生。

本方案不预先假定上述根因排序，而是**先用验证设施量化**，再分阶段落地。

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

## 4. 共用基建（三个方案均依赖，须先落地）

### 4.1 玩家 XY 取法修正（前置）

**现状问题**：`CarCoordX`/`CarCoordZ`（`src/compute/items.rs:504` / `:524`）取 `other_cars.car_coordinates[0]` / `[2]`，**仅当 `player_car_id==0` 时正确**。`OtherCarsSample.car_coordinates` 是 60 车 × 3 的扁平数组（`src/types.rs:935`），玩家车须用 `player_car_id` 索引。

**正确取法**（已存在于 `src/trackmap.rs:82-90`，可抽公共）：
```
player_idx = other_cars.player_car_id as usize;
base = player_idx * 3;
x = car_coordinates[base];
z = car_coordinates[base + 2];
```

**落地**：
- 抽公共函数 `player_xz(other_cars: &OtherCarsSample) -> Option<(f32,f32)>`（放 `src/trackmap.rs` 或新 `src/compute/util.rs`）。
- 修正 `CarCoordX`/`CarCoordZ` 复用之（顺手修 latent bug）。
- 新 delta item 内部直接调用，不依赖 `car_x`/`car_z`。

### 4.2 S/F 亚帧对齐（F-sub，必做）

**目的**：消除整圈恒定偏置，±10ms 前提。

**算法**：跨线两帧 `(t0,p0,v0)`、`(t1,p1,v1)`（`p1` 突降），在两帧间用速度感知插值求精确过线时刻 `t_cross` 与对应 `s_cross`：
- 线性近似：`t_cross = t0 + (t1−t0) * (p0 / (p0 + (1−p1)))`（position 域线性）；
- 速度感知（更准）：以 `v` 线性积分 `Δs` 反推过线比例。
- 当前圈 `s` 积分从 `t_cross` 起算，`t_cur = i_current_time − t_cross`；参考圈表 `t_best(s)` 的 `s=0` 对齐到 `t_cross` 处。

**判据组合**（取代单一 position 启发式）：
- 主判据：`completed_laps` 跳变（最干净）；
- 辅判据：`i_current_time` 归零、`normalized_car_position` 跨 0.8→0.2；
- 三者中任一触发即候选，取一致的那次。

**落地**：参考圈预算与实时 `s` 估计器初始化共用一个 `sf_align.rs` 工具模块。

### 4.3 验证设施（G-replay，精度优先须先搭）

**离线回放 harness**（落 `tests/compute_tests.rs` 新 `delta_accuracy` 模块）：
1. 取一 `.acctlm2` 文件的两圈高质量 flying lap A、B；
2. 各用同一方法建精确 `(s,t)` 表；
3. 以 A 为参考算 `δ_AB(s)`，以 B 为参考算 `δ_BA(s)`，自洽检验 `δ_AB(s) ≡ −δ_BA(s)`；
4. 与现有 `interpolated` 叠加，画 δ-vs-s 分歧段定位偏差源。

**逐帧 dump**：`(sample_tick, s_cur, t_cur, matched_seg, s_best, t_best, ref_v, δ)` 落 CSV，供离线绘图。

**运行时计量**：`process_frame` 已记 `last_compute_duration_ns`/`max_compute_duration_ns`（`src/dashboard/service.rs:640-648`），直接复用验证实时预算。

---

## 5. 参考圈重建：R-int 速度感知插值（共用）

**核心公式**：参考圈相邻点 `(s0,t0,v0)`、`(s1,t1,v1)`，假设 `v(s)` 线性，则
```
t(s) = t0 + (s1−s0)/(v1−v0) * ln( (v0 + (v1−v0)*r) / v0 )
其中 r = (s − s0) / (s1 − s0)
```
退化（`|v1−v0|` 极小）：用梯形 `t(s) ≈ t0 + 2*(s−s0)/(v0+v1)`。

**直接消除"恒速假设"偏差**，只多读一个 `velocity`。

**预重采样（R-resample）**：用 R-int 把参考圈重采样到密集均匀 `s` 网格（`N=16384` → 5km 约 0.3m/格；或 `65536` → 0.076m/格，内存 512KB/圈，可忽略），存为 `Vec<f32>` 量化表。实时只剩二分或直接下标。

---

## 6. 方案 A — s-R 参考折线投影（推荐先行）

**思路**：参考圈自身的 XZ 轨迹即带弧长的折线——它就是"中线"。实时把当前 XY 投影到参考折线得 `s`，查 `t_best[s]`，`δ = i_current_time − t_best`。**无需每赛道建表**。

### 6.1 数据结构（参考圈更新时预算，内存）

```rust
struct RefPoly {
    pts: Vec<(x: f32, z: f32, t_ms: f32, v: f32)>,  // 参考圈每帧
    cumlen: Vec<f32>,                                 // cumlen[i] = 到 pts[i] 的累计弧长
    total_len: f32,
    // 量化查表（可选，把 O(log n) 压到 O(1)）
    t_best_bins: Vec<f32>,                            // 长度 N，t_best[bin] = 参考圈在 bin*step 的时刻
    bin_step: f32,
    signature: (usize, usize, u64, u64),              // (ptr,len,first_tick,last_tick) 沿用 items.rs:404
}
```

### 6.2 构建（新最佳圈触发，可重）

1. 对参考圈每帧取 `(x,z, i_current_time, speed=hypot(v[0],v[2]))`；
2. 去静止点（`v<ε`）、1–2 帧去抖平滑；
3. S/F 亚帧对齐（§4.2）确定 `s=0`；
4. `cumlen` 由相邻点欧氏距离累加；保证 `t` 随 `cumlen` 单调（取累计 max 修正回退）；
5. 用 R-int 把 `(cumlen, t, v)` 重采样到均匀 `s` 网格 → `t_best_bins`。

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

### 6.4 代码落点

- 新 struct `DeltaTimeToSessionBestRefPoly`（`src/compute/items.rs`），impl `RealtimeComputeItem`。
- 字段：`RefPoly` + `last_seg: usize` + `last_lap: i32` + 输出 median 短窗。
- 注册：加入 `all_builtin_calculated_items()`（`src/compute/items.rs:578`）与默认 `ComputeRegistry` 初始化。
- 参考圈来源：`ctx.reference_lap`（已由 `ComputeRegistry::replace_reference` 注入 session-best，`src/compute/registry.rs:249`）。
- 投影/取坐标公共函数放 `src/compute/util.rs`，trackmap.rs 复用。

### 6.5 精度 / 速度 / 风险

- **精度**：`s` 误差 ≈ 参考圈采样间距（好采样亚米）+ 投影误差（亚米）；R-int 消除恒速偏差；F-sub 消除恒定偏置。可达 ±10ms 量级。
- **速度**：投影扫 ~9 段 + 一次数组读 ≈ 数百 ns。
- **风险**：参考圈走线与当前圈差异大时（spin/不同 racing line），局部搜索可能错段 → 加置信度门控（投影距离超阈则冻结 δ 并标记）。
- **优点**：开箱即用、无赛道表；参考圈换时自动重建。

---

## 7. 方案 B — s-C 赛道中线 + 量化表（长期最稳）

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

参考圈 `t_best` 量化表同方案 A 的 `t_best_bins`，但 `s` 域是赛道中线弧长（参考圈帧投影到中线得 `s_ref`，再 R-int 重采样）。

### 7.2 构建

**赛道中线（每赛道一次）**：
- 首次见到某 `track_name`：取第一圈有效 flying lap 的 XZ 轨迹；
- 去重、等弧长重采样到 1m（或 0.3m 加密）；
- 算 `cumlen`；落盘 `trackmaps/<track_id>.bin` 复用。
- 复用 `src/trackmap.rs` 的 `extract_track_coordinates`（:48）+ `detect_lap_crossings`（:216）。

**参考圈表（换最佳圈时）**：
- 参考圈每帧投影到中线得 `s_ref` + `t=i_current_time` + `v`；
- R-int 重采样到均匀 `s` 网格 → `t_best_bins`；
- F-sub 对齐 `s=0` 到起跑线。

### 7.3 实时 compute()

与方案 A 几乎相同，区别：投影目标是 `Centerline` 而非 `RefPoly`；`s` 域是赛道弧长。其余步骤（查表、δ、clamp、median）一致。

### 7.4 代码落点

- 新 struct `DeltaTimeToSessionBestCenterline`（`src/compute/items.rs`）。
- 中线构建/存取：新模块 `src/trackmap/centerline.rs`（或扩 `src/trackmap.rs`），按 `track_id` 缓存 + 磁盘复用。
- 需要从 `ComputeContext` 获取 `track_id`：确认 `current_frame` 是否携带 `track_name`（若否，需经 `metadata` 传入；见 §11 开放问题）。

### 7.5 精度 / 速度 / 风险

- **精度**：`s` 误差 ≈ 中线分辨率（1m → 可加密 0.3m）；R-int + F-sub 同方案 A。长期最稳，独立于单圈走线噪声。
- **速度**：同方案 A（O(1)）。
- **风险/成本**：需每赛道一次性建表 + 磁盘管理；中线 `s=0` 须与 S/F 对齐（F-center）。
- **何时上**：方案 A 验证后若走线差异扰动大或精度仍不足，升级到 B。

---

## 8. 方案 C — t 域反向表 + 速度门控混合（可选极速，语义改变）

**思路**：参考圈存"圈内时刻 t → 参考圈位置 `s_best(t)`"，`i_current_time` 即实时下标，零搜索。δ 语义变为"距离差/速度"。

### 8.1 数据结构

```rust
struct InvTable {
    s_at_t: Vec<f32>,   // 按 1ms 一格，一圈 ~120s → 120000 格 ~1MB；或按原始 (t,s) 二分
}
// 同时保留方案 A/B 的 t_best_bins（s 域）用于低速段切换
```

### 8.2 构建

参考圈 `(i_current_time, s_ref)`（`s_ref` 来自中线或参考折线投影），R-int 或 PCHIP 插值到均匀 `t` 网格 → `s_at_t`。

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

- **高速段**：零搜索、零 s 匹配误差（匹配在 t 域，t 已知精确），最响应。
- **低速弯**：t 域病态（`v→0` 除法爆炸）→ 切 s 域。
- **速度**：两次 O(1) 查表 + 一次混合，最快档。
- **代价/风险**：**语义改变**——高速段报"距离差折算时间"，与"同位置时间差"在弯道进出口可能差几十 ms，overlay 上有跳变风险（需平滑过渡）。
- **定位**：不作主 delta；仅在"语义可换且要极致响应"时作为并行 `delta_time_to_session_best_fast` 项。

---

## 9. 输出鲁棒性（所有方案共用）

- **O-clamp**：`|δ|` 限幅 ±5s，杀异常。
- **O-median**：短窗（3–5 帧）中值 + 滞回，杀单帧尖刺不引入滞后。
- **O-conf**：低速弯 `v` 小 → 同 `s` 误差放大为更大时间误差，附置信度标记（可由 dashboard 侧弱化显示）。
- **O-hold**：`s` 估计不确定（出界/投影大跳/`is_in_pit`）时短暂冻结 δ。

---

## 10. 分阶段落地路线

| 阶段 | 内容 | 验证门槛 |
|---|---|---|
| **P0 基建** | §4.1 玩家 XY 修正 + §4.3 G-replay harness + §4.2 F-sub 工具 + §5 R-int | dump 现有两方法 δ-vs-s，量化偏差形态 |
| **P1 方案 A** | `DeltaTimeToSessionBestRefPoly`（R-int + 参考折线投影 + 量化表 + F-sub + O-clamp/median） | G-replay 自洽 `δ_AB=−δ_BA`；弯道 S 曲线偏差消除；`compute()` <1ms |
| **P2 评估** | 对比 A 与现有 interpolated 的 δ-vs-s，看是否达标 | 局部 ±10ms 量级 |
| **P3 方案 B（条件）** | 若 A 走线差异扰动大或精度不足 → 上中线 s-C | 同 P1 门槛 + 跨圈/跨走线稳定 |
| **P4 方案 C（可选）** | 若需极致响应且接受语义切换 → 并行 fast 项 | 语义一致性评审通过 |

每阶段可独立评审与合并；P1 不依赖 P3 的赛道表。

---

## 11. 开放问题（评审时定）

1. **`track_id` 获取**：方案 B 需在 `compute()` 内知道当前赛道。`ComputeContext` 当前不携带 `metadata.track_name`。选项：
   - (a) `ComputeRegistry` 持有当前 `track_id`，item 注册时注入；
   - (b) 扩 `ComputeContext` 增加 `track_id` 字段；
   - (c) 方案 B 延后，先用方案 A 规避此问题。
2. **`i_current_time` 起点一致性**：ACC 的 `i_current_time` 是否每圈从 0 起算且与 S/F 物理过线对齐？需用 G-replay 在 §4.2 中实测确认；若不一致，F-sub 需补偿。
3. **参考圈走线 vs 当前圈走线差异阈值**：方案 A 投影置信度门控的阈值需实测标定。
4. **是否保留现有两个 item**：P1 上线后是替换 `..._interpolated` 还是新增 `_refpoly` 并行？倾向新增并行→验证后废弃旧项。
5. **`Bin` 量化粒度选型**：16384 vs 65536，按 5km 赛道实测精度差 vs 内存（128KB vs 512KB，均可忽略）。

---

## 12. 评审检查清单

- [ ] 根因诊断假设（恒速插值/position 分辨率/S-F 对齐）是否认同，是否需先 P0 量化再选型？
- [ ] delta 语义保持"同位置时间差"（方案 A/B 为主）是否同意？
- [ ] §4.1 玩家 XY 修正是否同意顺手修 `car_x`/`car_z`？
- [ ] §4.2 S/F 判据组合（`completed_laps` 主 + position/time 辅）是否够稳？
- [ ] §6 方案 A 作为先行是否同意？是否直接上 §7 方案 B？
- [ ] §10 分阶段路线与验证门槛是否接受？
- [ ] §11 开放问题 1（`track_id` 注入方式）倾向哪条？

---

## 附录 A — 探索的方案空间与取舍

> 本附录记录设计前的脑暴过程：考虑过哪些方向、为何取舍、为何收敛到 §6–§8 的 A/B/C 三方案。保留是为避免评审时重复争论已讨论过的方向，并显露决策依据。

### A.1 误差预算（硬约束，决定方案空间）

200 km/h ≈ 55 m/s，**10 ms ≈ 0.55 m**。而 `normalized_car_position` 分辨率按 0.001 计，5 km 赛道 ≈ 5 m/格——**仅 position 量化这一项就超预算 10 倍**。

推论：**任何只依赖 `normalized_car_position` 的方案在高速段物理上达不到分析级精度**。子米级 `s` 必须自行派生（速度积分 / 中线弧长投影）。这条约束直接排除了"继续在 position 域改良插值"的路线，把方案空间收窄到"换匹配域"。

### A.2 被考虑过的方向（按层分类）

脑暴时把问题拆成 5 个可独立选型的层，每层枚举候选：

| 层 | 候选 | 备注 |
|---|---|---|
| **匹配域 s** | s-P(position) · s-V(速度积分) · s-C(中线弧长) · s-H(粗门+细积分) · s-K(Kalman) · s-R(参考折线投影) | s-P 单用精度不够；其余可行 |
| **参考重建** | R-lin(线性) · R-int(速度感知) · R-pchip(单调三次) · R-resample(预重采样) · R-bspline(B 样条) · 普通三次样条 | 普通三次破坏单调，弃 |
| **实时 s 估计** | L-proj(投影) · L-int(积分) · L-kalman · L-gate(position 异常门) | 均可叠加 |
| **S/F 对齐** | F-wrap · F-lap · F-time · F-sub(亚帧) · F-center(中线零点) | F-sub 必备 |
| **输出鲁棒** | O-clamp · O-median · O-conf · O-hold | 全部纳入共用 |
| **备选语义** | S-point(同位置时间差) · S-arrival(前向到达) · S-mini(mini-sector) · S-virtual(虚拟最佳) · S-phase(弯道相位) · S-t-domain(距离差/速度) · DTW · 互相关 | 多数弃，见 A.4 |

### A.3 精度与速度同向（关键洞察）

实时热路径是 `process_frame`（`src/dashboard/service.rs:527`）每帧同步调用 `compute()`，单线程顺序。单 delta 无法并行，跨 item 并行对单 delta 无意义。

但所有精度赢家（R-int / R-resample / 量化表 / 中线投影）的**重活都在参考圈更新时一次性预算**（罕见、可重），实时 `compute()` 只剩一次投影 + 一次数组读 = O(1)，比现状的 O(n) 扫描还快。

**结论：精度与速度不冲突，反而同向。** "机器资源多"应投在预计算密度 / 空间索引 / 多估计器冗余，而非并行。这让"上 R-int + 量化表"没有实时性代价。

### A.4 被排除/降级的方向及理由

- **普通三次样条 / Akima**：对 `(s,t)` 拟合会产生过冲 wiggle，破坏 `t` 对 `s` 的物理单调性 → 时间倒流。改用 **R-pchip(Fritsch-Carlson)** 保单调，或直接 **R-int**（基于物理 `v`，不引入 wiggle）。最终选 R-int 为主，因它直接消除恒速假设偏差且只需多读一个 `velocity`。
- **DTW 轨迹对齐**：数学上"最优"非线性时间对齐，但非实时友好（O(n²)），且语义模糊。弃。
- **互相关全局对齐**：对速度/距离曲线做互相关只能给一个"整圈 delta"标量，无法逐点。仅适合 overlay 的平滑总差，不适合分析级逐点 δ。降级为"未来可选的 overlay 平滑项"。
- **S-mini(mini-sector 累计)**：极稳但精度低（分段粒度），广播级而非分析级。与"精度优先"目标冲突，降级为备选。
- **S-virtual(虚拟最佳圈)**：拼接各 mini-sector 历史最佳段做参考，去单圈噪声；但参考非真实圈，语义偏离"本 session 最佳圈"。降级为"参考圈降噪的可选增强"（即 §6.2 的平滑/共识轨迹思路，不单独立项）。
- **S-phase(弯道相位对齐)**：按刹车点/入弯点对齐而非 s，异想、无稳定判据。弃。
- **S-arrival(前向到达时间差)**：与 S-point 在 s=0 附近有别、更平滑，但语义不直观且需前向预测。暂不取，记为备选。
- **IMU 双积分类距离**：漂移过大，弃。
- **ACC 内置 delta / `gap_*` 字段**：`SessionSample.gap_ahead_or_tail_value`/`gap_behind` 是车间距，非圈 delta；ACC 内置 delta 已实测不准。均弃。

### A.5 t 域反向表的诱惑与代价（方案 C 的取舍）

`t 域反向表` `s_best(t)` 用 `i_current_time` 直接下标，**零搜索、零 s 匹配误差**，是所有方案里实时最快、高速段最响应的。但它的 δ 语义是 `(s_cur−s_best)/v`（距离差折算时间），**与"同位置时间差"在弯道进出口会差几十 ms**，overlay 上有跳变风险。

取舍：**不作主 delta**（语义不一致会让分析困惑），仅在"语义可换且要极致响应"时作为并行 fast 项（§8）。主 delta 仍用 s 域（方案 A/B），保留 S-point 语义。

### A.6 为何收敛到 A/B/C

从 A.2 的候选层组合中，按"精度优先 + 实时无忧 + 工程量递增"筛出三条主线：

1. **方案 A (s-R)**：参考折线投影 + R-int + 量化表。**免赛道建表、开箱即用**，精度上限相当，适合先行验证根因。隐含假设"参考圈走线≈当前走线"，加置信度门控可扛小差异。
2. **方案 B (s-C)**：赛道中线 + 量化表。`s` 域独立于任何单圈走线，**长期最稳**，但需每赛道一次性建表。作为 A 验证后的升级路径。
3. **方案 C (t 域)**：极速但语义改变，**仅作可选并行项**。

三者共享 §4 基建（玩家 XY 修正 / F-sub / G-replay）与 §5 R-int。A→B 是"从轻到重、每步可量化验证"的递进，避免一上来背赛道建表负担；C 不阻塞主线。

其余候选（PCHIP/B 样条/Kalman/双估计器/共识轨迹等）作为各层内的**可选增强**保留，不单独成方案——例如 PCHIP 可在 R-int 不够平滑时替换参考重建内插；Kalman 可在实时 s 抖动大时替换 L-proj；共识轨迹可在参考圈噪声大时作为参考源。这些都在 §6–§9 的对应层留有接口，无需预先定案。
