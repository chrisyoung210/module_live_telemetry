# Delta Time 精度优化方案 Review 反馈

> **被 Review 文档**: `2026-07-03-delta-time-session-best-redesign.md`  
> **Review 日期**: 2026-07-04  
> **Reviewer**: OpenCode  
> **状态**: 待被 Review 方确认 / 讨论  

---

## 1. 总体评价

方向基本正确：当前 `interpolated` 版本的 delta 不准，根因确实不在参考圈内部的线性插值，而在**匹配域太粗**——`normalized_car_position` 0.001 的分辨率在 5 km 赛道上约等于 5 m/格，高速段会放大成几十毫秒误差。

把 delta 计算从 **position 域** 迁移到 **弧长 s 域**，并用 XZ 世界坐标/中线投影来估计 `s`，是提升精度的正确路径。

但文档中有几处技术细节需要修正，尤其是 **§5 的 R-int 公式** 存在数学不一致，不能原样作为共用基建。建议先落地方案 A，但前提是先搭起量化验证设施，用数据说话。

---

## 2. 当前代码快速核对

已核对相关源码：

- `src/compute/items.rs:316` — `DeltaTimeToSessionBestLap`：离散扫描，直接相减。
- `src/compute/items.rs:385` — `DeltaTimeToSessionBestLapInterpolated`：单调化 `(position, time)` 后二分线性插值。
- `src/compute/items.rs:504` / `:524` — `CarCoordX`/`CarCoordZ` 取 `car_coordinates[0]`/`[2]`，存在 `player_car_id` 索引 bug。
- `src/trackmap.rs:82-90` — 已存在正确的 `player_car_id` 取法。
- `src/compute/context.rs` — `ComputeContext` 当前不携带 `track_name`/`track_id`。

---

## 3. 逐条反馈

### 3.1 §4.1 玩家 XY 取法修正 — ✅ 同意，优先级最高

当前 `CarCoordX`/`CarCoordZ` 假设玩家车永远是索引 0，这是明确的 latent bug。

**建议落地**:
- 新增公共函数，例如 `src/compute/util.rs`：

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

- 顺手修正 `CarCoordX`/`CarCoordZ`，并补单元测试。

---

### 3.2 §4.2 S/F 亚帧对齐（F-sub）— ✅ 必要，但建议先验证 `i_current_time` 语义

这是 ±10 ms 目标的关键。但有个前置问题要先确认：

> **`i_current_time` 是否每圈从 0 开始、且物理过 S/F 线时归零？**

- 如果是，方案里的 `t_cur` 可以直接用 `i_current_time`；
- 如果不是，整个 delta 语义都需要重新对齐。

**建议 P0 先做**: 用 G-replay 读真实 `.acctlm2`，看同一圈 S/F 处 `i_current_time` 和 `normalized_car_position` 的跳变规律，再决定 F-sub 需要多深。

F-sub 公式中的 position 线性插值：

```
t_cross = t0 + (t1−t0) * (p0 / (p0 + (1−p1)))
```

可以作为 baseline；若需要更准，再结合速度做线性插值。

---

### 3.3 §4.3 验证设施（G-replay）— ✅ 最关键，必须先做

没有量化验证，无法判断 A/B/C 哪个好。

建议 harness 先回答三个问题：

1. 现有 `interpolated` 方法的误差形态是什么？（弯道是否有 S 形系统偏差？直道是否稳定？）
2. 方案 A 相比 `interpolated`，弯道 δ 波动是否显著降低？
3. 自洽检验 `δ_AB(s) = −δ_BA(s)` 是否成立？

**这是后续所有技术决策的前提**。

---

### 3.4 §5 R-int 速度感知插值 — ⚠️ 公式有 bug，不能直接用

这是文档里最严重的问题。

给出的公式：

```
t(s) = t0 + (s1−s0)/(v1−v0) * ln( (v0 + (v1−v0)*r) / v0 )
r = (s − s0) / (s1 − s0)
```

**问题分析**:

当 `r = 1` 时：

```
t(s1) = t0 + (s1−s0)/(v1−v0) * ln(v1/v0)
```

这**并不等于 `t1`**，除非该段恰好满足 `t1−t0 = (s1−s0)/(v1−v0) * ln(v1/v0)`。一般情况下不成立。

也就是说，这个 R-int 虽然自称“速度感知”，但并不过实际采样点 `(s1, t1)`，会导致参考圈总时间失真和系统性偏差。**不能作为共用基建**。

**建议替代方案**:

1. **最简单且足够好**：直接线性插值 `(s, t)`。ACC 采样率 50–100 Hz，一帧约 1–2 m，线性误差远小于当前 position 域 5 m 的量化误差。
2. **更平滑**：用 monotonic PCHIP 插值 `(s, t)`，避免 wiggle 且过所有采样点。
3. 若一定要引入速度信息，应使用速度做**采样点密度控制**或**异常值剔除**，而不是作为 `t(s)` 的模型。

**结论**：不要单独为 R-int 建模块。先上线性 `(s,t)` 插值的方案 A，后面如果觉得不够平滑再换 PCHIP。

---

### 3.5 §6 方案 A（s-R 参考折线投影）— ✅ 推荐作为 P1

性价比最高的第一步。但有几个注意点：

#### 3.5.1 参考折线作为“中线”是有偏的

方案 A 的 `s` 不是赛道几何中线，而是**参考圈那一条走线的弧长**。如果当前圈和参考圈走线不同（入弯点、出弯点、spin 后回赛道），投影到参考折线会产生两类问题：

- 同一弯道，不同走线的 `s` 不对应同一物理位置；
- 投影距离大时，最近点可能跳到其他段，delta 会跳。

文档提到“置信度门控”，建议明确：
- 投影距离 > 阈值（如 3–5 m）→ 输出 `NoValidData` 或 hold 上一帧；
- 局部搜索窗口失效时 fallback 到全局最近点。

#### 3.5.2 `project_local` 实现细节

扫 `[last_seg−4, last_seg+4]` 约 9 段，需注意：
- 跨 S/F 时窗口要 wrap；
- 高速/低帧率下 9 段可能不够，建议按“最大帧间弧长”动态覆盖，比如 100–200 ms；
- 用 **2D 点到线段距离** 投影，不是点到点。

#### 3.5.3 `t_best_bins` bin 数

16384 vs 65536 对 5 km 赛道分别是约 0.3 m / 0.08 m，都够用。内存差 128 KB vs 512 KB，可忽略。

**建议**：先用 65536，精度更富裕。

#### 3.5.4 s 的构建

- 去掉静止/低速点（`v < ε`），避免 pit 或 grid 阶段噪声；
- S/F 处确保 `s=0` 对齐，不要简单把两段折线拼起来。

---

### 3.6 §7 方案 B（s-C 赛道中线）— ✅ 长期最稳，但建议后置

理论上最正确，因为 `s` 是赛道几何属性，独立于任何单圈走线。

**关于 §11 开放问题 1：`track_id` 注入**

建议选 **(b) 扩展 `ComputeContext` 增加 `track_name`/`track_id` 字段**。

- `TelemetryFrame` 里没有赛道名，但 metadata 有；
- `DashboardService` 初始化或收到 metadata 后注入即可；
- 成本最低，方案 A 暂时用不到也能先加上。

中线持久化 `trackmaps/<track_id>.bin` 需要额外工作（文件管理、版本控制、首次生成 UX），建议等方案 A 验证后，确认走线差异真的造成 ±10 ms 以上误差再上 B。

---

### 3.7 §8 方案 C（t 域反向表）— ⚠️ 不建议作为主线

这个方案改变了 delta 语义：

- A/B: `δ(s) = t_cur(s) − t_best(s)`，同位置时间差；
- C: 高速段变成 `(s_cur − s_best) / v_cur`，距离差折算时间；低速段又切回 s 域。

这会在弯道进出口造成语义跳变，overlay 很可能闪烁。除非有非常明确的用例，否则不建议作为 dashboard 主 delta。

如果一定要做，建议作为单独 item（如 `delta_time_to_session_best_lap_fast`），并明确标注语义不同。

---

### 3.8 §9 输出鲁棒性 — ✅ 同意，但保持轻量

- **O-clamp ±5s**: 同意。
- **O-median 3–5 帧**: 同意，但注意延迟。3 帧中值约 30–60 ms 可接受；5 帧可能偏多。
- **O-conf / O-hold**: 先做简单版，投影距离大和低速时输出 `NoValidData` 即可，不必做复杂状态机。

---

## 4. 修正后的落地顺序

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **P0** | 1. 玩家 XY 取法修正<br>2. 搭建 G-replay harness（读 `.acctlm2`、选两圈、输出 δ-vs-s CSV、自洽检验）<br>3. 用现有 `interpolated` 跑基准误差曲线 | 有现有方法的误差形态图 |
| **P1** | 实现方案 A：参考折线投影 + 均匀 s 网格 + 线性 `(s,t)` 插值 + 简单 F-sub + O-clamp | G-replay 自洽 `δ_AB = −δ_BA`；弯道 S 形偏差显著减小；`compute()` < 1 ms |
| **P2** | 评估：若 A 精度达标，新增/并行 `interpolated` 替代；若走线差异大，升级到 B | 局部 ±10 ms |
| **P3** | 方案 B：赛道中线 + 持久化 | 跨走线稳定 |
| **P4** | 可选方案 C 作为极速项 | 语义评审通过 |

---

## 5. 最关键的几条建议

1. **先修 §4.1 的 XY bug**，安全且高回报。
2. **R-int 公式不能直接用**，先用线性 `(s,t)` 插值，后面不够再换 PCHIP。
3. **G-replay 是决策前提**，不要跳过。没有它无法判断 A/B/C 哪个更好。
4. **方案 A 作为 P1 最合理**，但要清楚它的理论上限受参考折线 bias 限制。
5. **不要替换现有 `interpolated`**，先新增 `delta_time_to_session_best_lap_refpoly` 并行验证。
6. **track_id 注入选 (b)**，扩展 `ComputeContext` 成本最低。

---

## 6. 如果现在就开干

建议先开两个 PR：

1. **PR 1: Bugfix** — 修正 `CarCoordX`/`CarCoordZ` 的 `player_car_id` 索引，并抽公共 `player_xz`。
2. **PR 2: Delta accuracy harness** — 在 `tests/compute_tests.rs` 新增离线回放测试，能读取 `.acctlm2`、取两圈、输出 δ-vs-s CSV，并跑通自洽检验 `δ_AB = −δ_BA`。

等 harness 跑出现有 `interpolated` 的 baseline 数据后，再决定方案 A 的实现细节。
