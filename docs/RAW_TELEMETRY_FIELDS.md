# ACC Raw Telemetry Field Map

> Status: current implementation reference (updated 2026-06-03).
> Struct: fixed `SPageFileGraphicsRaw` with `penalty` field at offset `1228`.
> Format: ACC raw shared-memory page dump (`.accraw` file format).
> Scope: fields saved by `record-raw` in `module_live_telemetry`.

## Summary

Each recorded raw sample stores three byte-for-byte shared-memory pages plus two recorder clock fields:

| Item | Bytes | Flattened fields | Meaning / 含义 |
|---|---:|---:|---|
| `sampleTick` | 8 | 1 | Recorder sample counter / 录制样本序号 |
| `timestampNs` | 8 | 1 | Recorder monotonic timestamp in ns / 录制单调时间戳，纳秒 |
| `rawPhysicsPage` | 800 | 200 | `Local\acpmf_physics` raw page / 物理共享内存原始页 |
| `rawGraphicsPage` | 1588 | 86 | `Local\acpmf_graphics` raw page / 图形共享内存原始页 |
| `rawStaticPage` | 200 | 98 | current static identity prefix from `Local\acpmf_static` / 当前实现保存的静态身份信息前缀 |

Notes:

- Offset is byte offset from the beginning of that raw page, not from the ACTL file.
- Offset values are verified against the actual `SPageFileGraphicsRaw` struct compiled with `repr(C)` and `pack=4`.
- `is_valid_lap` is at graphics offset `1408`.
- The `penalty` field (AC_PENALTY_TYPE) at offset `1228` was previously missing from the struct. The old code's `penalty_type` at 1224 was actually the `flag` field. This caused a 4-byte offset shift for all subsequent struct fields. The ACC memory layout itself is unchanged; only the struct field mapping is corrected.

## Physics Page: `rawPhysicsPage`

| Field | Type | Count | Offset | Length | 中文含义 | English meaning |
|---|---|---|---:|---:|---:|---|---|
| `packet_id` | `i32` | 1 | 0 | 4 | 物理页数据包序号，用于检测新帧 | Physics packet sequence id for detecting new frames |
| `gas` | `f32` | 1 | 4 | 4 | 油门踏板原始比例 | Raw throttle pedal ratio |
| `brake` | `f32` | 1 | 8 | 4 | 刹车踏板原始比例 | Raw brake pedal ratio |
| `fuel` | `f32` | 1 | 12 | 4 | 剩余燃油 | Remaining fuel |
| `gear` | `i32` | 1 | 16 | 4 | ACC 原始档位编码 | ACC raw gear code |
| `rpms` | `i32` | 1 | 20 | 4 | 发动机转速 | Engine RPM |
| `steer_angle` | `f32` | 1 | 24 | 4 | 方向盘原始比值 | Raw steering ratio |
| `speed_kmh` | `f32` | 1 | 28 | 4 | 车速 km/h | Vehicle speed in km/h |
| `velocity` | `f32` | 3 | 32 | 12 | 速度向量 | Velocity vector |
| `acc_g` | `f32` | 3 | 44 | 12 | G 力向量 | G-force vector |
| `wheel_slip` | `f32` | 4 | 56 | 16 | 四轮滑移值 | Wheel slip per wheel |
| `wheel_load` | `f32` | 4 | 72 | 16 | 四轮载荷 | Wheel load per wheel |
| `wheels_pressure` | `f32` | 4 | 88 | 16 | 四轮胎压 | Tyre pressure per wheel |
| `wheel_angular_speed` | `f32` | 4 | 104 | 16 | 四轮角速度 | Wheel angular speed per wheel |
| `tyre_wear` | `f32` | 4 | 120 | 16 | 四轮轮胎磨损 | Tyre wear per wheel |
| `tyre_dirty_level` | `f32` | 4 | 136 | 16 | 四轮轮胎脏污程度 | Tyre dirt level per wheel |
| `tyre_core_temperature` | `f32` | 4 | 152 | 16 | 四轮胎芯温度 | Tyre core temperature per wheel |
| `camber_rad` | `f32` | 4 | 168 | 16 | 四轮外倾角弧度 | Camber angle in radians per wheel |
| `suspension_travel` | `f32` | 4 | 184 | 16 | 四轮悬挂行程 | Suspension travel per wheel |
| `drs` | `f32` | 1 | 200 | 4 | DRS 原始状态/值 | Raw DRS state/value |
| `tc` | `f32` | 1 | 204 | 4 | 物理页牵引力控制原始值 | Physics-page traction control raw value |
| `heading` | `f32` | 1 | 208 | 4 | 车头朝向 | Vehicle heading |
| `pitch` | `f32` | 1 | 212 | 4 | 俯仰角 | Pitch angle |
| `roll` | `f32` | 1 | 216 | 4 | 侧倾角 | Roll angle |
| `cg_height` | `f32` | 1 | 220 | 4 | 重心高度 | Center of gravity height |
| `car_damage` | `f32` | 5 | 224 | 20 | 车体损伤数组 | Car body damage array |
| `number_of_tyres_out` | `i32` | 1 | 244 | 4 | 超出赛道边界的轮胎数量 | Number of tyres outside track limits |
| `pit_limiter_on` | `i32` | 1 | 248 | 4 | 维修区限速器原始状态 | Raw pit limiter state |
| `abs` | `f32` | 1 | 252 | 4 | 物理页 ABS 原始值 | Physics-page ABS raw value |
| `kers_charge` | `f32` | 1 | 256 | 4 | KERS 电量 | KERS charge |
| `kers_input` | `f32` | 1 | 260 | 4 | KERS 输入 | KERS input |
| `auto_shifter_on` | `i32` | 1 | 264 | 4 | 自动换挡原始状态 | Raw auto shifter state |
| `ride_height` | `f32` | 2 | 268 | 8 | 车身高度数组 | Ride height array |
| `turbo_boost` | `f32` | 1 | 276 | 4 | 涡轮增压值 | Turbo boost |
| `ballast` | `f32` | 1 | 280 | 4 | 配重 | Ballast |
| `air_density` | `f32` | 1 | 284 | 4 | 空气密度 | Air density |
| `air_temp` | `f32` | 1 | 288 | 4 | 环境气温 | Air temperature |
| `road_temp` | `f32` | 1 | 292 | 4 | 路面温度 | Road temperature |
| `local_angular_vel` | `f32` | 3 | 296 | 12 | 本地角速度向量 | Local angular velocity vector |
| `final_ff` | `f32` | 1 | 308 | 4 | 最终力反馈值 | Final force feedback value |
| `performance_meter` | `f32` | 1 | 312 | 4 | 性能表原始值 | Performance meter raw value |
| `engine_brake` | `i32` | 1 | 316 | 4 | 发动机制动设置 | Engine brake setting |
| `ers_recovery_level` | `i32` | 1 | 320 | 4 | ERS 回收等级 | ERS recovery level |
| `ers_power_level` | `i32` | 1 | 324 | 4 | ERS 输出等级 | ERS power level |
| `ers_heat_charging` | `i32` | 1 | 328 | 4 | ERS 热充电状态 | ERS heat charging state |
| `ers_is_charging` | `i32` | 1 | 332 | 4 | ERS 是否正在充电 | ERS charging state |
| `kers_current_kj` | `f32` | 1 | 336 | 4 | KERS 当前能量 kJ | Current KERS energy in kJ |
| `drs_available` | `i32` | 1 | 340 | 4 | DRS 是否可用 | DRS availability |
| `drs_enabled` | `i32` | 1 | 344 | 4 | DRS 是否启用 | DRS enabled state |
| `brake_temp` | `f32` | 4 | 348 | 16 | 四轮刹车温度 | Brake temperature per wheel |
| `clutch` | `f32` | 1 | 364 | 4 | 离合器原始比例 | Raw clutch ratio |
| `tyre_temp_i` | `f32` | 4 | 368 | 16 | 四轮轮胎内侧温度 | Inner tyre temperature per wheel |
| `tyre_temp_m` | `f32` | 4 | 384 | 16 | 四轮轮胎中部温度 | Middle tyre temperature per wheel |
| `tyre_temp_o` | `f32` | 4 | 400 | 16 | 四轮轮胎外侧温度 | Outer tyre temperature per wheel |
| `is_ai_controlled` | `i32` | 1 | 416 | 4 | 是否由 AI 控制 | AI controlled state |
| `tyre_contact_point` | `f32` | 12 | 420 | 48 | 四轮接地点坐标 | Tyre contact point vectors per wheel |
| `tyre_contact_normal` | `f32` | 12 | 468 | 48 | 四轮接地法线向量 | Tyre contact normal vectors per wheel |
| `tyre_contact_heading` | `f32` | 12 | 516 | 48 | 四轮接地方向向量 | Tyre contact heading vectors per wheel |
| `brake_bias` | `f32` | 1 | 564 | 4 | 刹车平衡 | Brake bias |
| `local_velocity` | `f32` | 3 | 568 | 12 | 本地速度向量 | Local velocity vector |
| `p2p_activations` | `i32` | 1 | 580 | 4 | Push-to-pass 激活次数 | Push-to-pass activation count |
| `p2p_status` | `i32` | 1 | 584 | 4 | Push-to-pass 状态 | Push-to-pass status |
| `current_max_rpm` | `i32` | 1 | 588 | 4 | 当前最大转速 | Current maximum RPM |
| `mz` | `f32` | 4 | 592 | 16 | 四轮回正力矩 | Aligning torque Mz per wheel |
| `fx` | `f32` | 4 | 608 | 16 | 四轮纵向力 | Longitudinal force Fx per wheel |
| `fy` | `f32` | 4 | 624 | 16 | 四轮横向力 | Lateral force Fy per wheel |
| `slip_ratio` | `f32` | 4 | 640 | 16 | 四轮滑移率 | Slip ratio per wheel |
| `slip_angle` | `f32` | 4 | 656 | 16 | 四轮滑移角 | Slip angle per wheel |
| `tc_in_action` | `i32` | 1 | 672 | 4 | TC 是否介入 | Traction control active state |
| `abs_in_action` | `i32` | 1 | 676 | 4 | ABS 是否介入 | ABS active state |
| `suspension_damage` | `f32` | 4 | 680 | 16 | 四轮悬挂损伤 | Suspension damage per wheel |
| `tyre_temp` | `f32` | 4 | 696 | 16 | 四轮综合胎温 | Combined tyre temperature per wheel |
| `water_temp` | `f32` | 1 | 712 | 4 | 水温 | Water temperature |
| `brake_pressure` | `f32` | 4 | 716 | 16 | 四轮刹车压力 | Brake pressure per wheel |
| `front_brake_compound` | `i32` | 1 | 732 | 4 | 前刹车配方 | Front brake compound |
| `rear_brake_compound` | `i32` | 1 | 736 | 4 | 后刹车配方 | Rear brake compound |
| `pad_life` | `f32` | 4 | 740 | 16 | 四轮刹车片寿命 | Brake pad life per wheel |
| `disc_life` | `f32` | 4 | 756 | 16 | 四轮刹车盘寿命 | Brake disc life per wheel |
| `ignition_on` | `i32` | 1 | 772 | 4 | 点火开关状态 | Ignition state |
| `starter_engine_on` | `i32` | 1 | 776 | 4 | 启动机状态 | Starter engine state |
| `is_engine_running` | `i32` | 1 | 780 | 4 | 发动机是否运行 | Engine running state |
| `kerb_vibration` | `f32` | 1 | 784 | 4 | 路肩震动反馈 | Kerb vibration feedback |
| `slip_vibrations` | `f32` | 1 | 788 | 4 | 滑移震动反馈 | Slip vibration feedback |
| `g_vibrations` | `f32` | 1 | 792 | 4 | G 力震动反馈 | G-force vibration feedback |
| `abs_vibrations` | `f32` | 1 | 796 | 4 | ABS 震动反馈 | ABS vibration feedback |

## Graphics Page: `rawGraphicsPage`

| Field | Type | Count | Offset | Length | 中文含义 | English meaning |
|---|---|---|---:|---:|---:|---|---|
| `packet_id` | `i32` | 1 | 0 | 4 | 图形页数据包序号 | Graphics packet sequence id |
| `status` | `i32` | 1 | 4 | 4 | 游戏状态 | Game status |
| `session` | `i32` | 1 | 8 | 4 | Session 类型 | Session type |
| `current_time` | `u16` | 15 | 12 | 30 | 当前圈时间字符串 | Current lap time string |
| `last_time` | `u16` | 15 | 42 | 30 | 上一圈时间字符串 | Last lap time string |
| `best_time` | `u16` | 15 | 72 | 30 | 最佳圈时间字符串 | Best lap time string |
| `split` | `u16` | 15 | 102 | 30 | 分段时间字符串 | Split time string |
| `completed_laps` | `i32` | 1 | 132 | 4 | 已完成圈数 | Completed laps |
| `position` | `i32` | 1 | 136 | 4 | 当前名次 | Current position |
| `i_current_time` | `i32` | 1 | 140 | 4 | 当前圈时间毫秒 | Current lap time in milliseconds |
| `i_last_time` | `i32` | 1 | 144 | 4 | 上一圈时间毫秒 | Last lap time in milliseconds |
| `i_best_time` | `i32` | 1 | 148 | 4 | 最佳圈时间毫秒 | Best lap time in milliseconds |
| `session_time_left` | `f32` | 1 | 152 | 4 | Session 剩余时间 | Remaining session time |
| `distance_traveled` | `f32` | 1 | 156 | 4 | 累计行驶距离 | Total distance traveled |
| `is_in_pit` | `i32` | 1 | 160 | 4 | 是否在维修区 | In-pit state |
| `current_sector_index` | `i32` | 1 | 164 | 4 | 当前赛道分段索引 | Current sector index |
| `last_sector_time` | `i32` | 1 | 168 | 4 | 上一分段时间 | Last sector time |
| `number_of_laps` | `i32` | 1 | 172 | 4 | Session 圈数 | Number of laps |
| `tyre_compound` | `u16` | 33 | 176 | 66 | 轮胎配方字符串 | Tyre compound string |
| `replay_time_multiplier` | `f32` | 1 | 244 | 4 | 回放时间倍率 | Replay time multiplier |
| `normalized_car_position` | `f32` | 1 | 248 | 4 | 归一化赛道位置 | Normalized car track position |
| `active_cars` | `i32` | 1 | 252 | 4 | 活跃车辆数量 | Active car count |
| `car_coordinates` | `f32` | 180 | 256 | 720 | 最多 60 台车坐标 | Coordinates for up to 60 cars |
| `car_id` | `i32` | 60 | 976 | 240 | 最多 60 台车 ID | IDs for up to 60 cars |
| `player_car_id` | `i32` | 1 | 1216 | 4 | 玩家车辆 ID | Player car id |
| `penalty_time` | `f32` | 1 | 1220 | 4 | 罚时 | Penalty time |
| `flag` | `i32` | 1 | 1224 | 4 | 旗语类型（蓝/黄/黑/白/方格/处罚） | Flag type (blue/yellow/black/white/chequered/penalty) |
| `penalty` | `i32` | 1 | 1228 | 4 | 处罚类型（DSQ等）；之前遗漏导致后续偏移-4 | Penalty type; was missing, caused 4-byte offset shift |
| `ideal_line_on` | `i32` | 1 | 1232 | 4 | 理想线是否开启 | Ideal line enabled state |
| `is_in_pit_lane` | `i32` | 1 | 1236 | 4 | 是否在维修通道 | In pit lane state |
| `surface_grip` | `f32` | 1 | 1240 | 4 | 路面抓地力 | Surface grip |
| `mandatory_pit_done` | `i32` | 1 | 1244 | 4 | 强制进站是否完成 | Mandatory pit completed state |
| `wind_speed` | `f32` | 1 | 1248 | 4 | 风速 | Wind speed |
| `wind_direction` | `f32` | 1 | 1252 | 4 | 风向 | Wind direction |
| `is_setup_menu_visible` | `i32` | 1 | 1256 | 4 | 设置菜单是否可见 | Setup menu visible state |
| `main_display_index` | `i32` | 1 | 1260 | 4 | 主显示页面索引 | Main display index |
| `secondary_display_index` | `i32` | 1 | 1264 | 4 | 副显示页面索引 | Secondary display index |
| `tc` | `i32` | 1 | 1268 | 4 | TC 等级 | Traction control level |
| `tc_cut` | `i32` | 1 | 1272 | 4 | TC Cut 等级 | Traction control cut level |
| `engine_map` | `i32` | 1 | 1276 | 4 | 引擎 Map | Engine map |
| `abs` | `i32` | 1 | 1280 | 4 | ABS 等级 | ABS level |
| `fuel_x_lap` | `f32` | 1 | 1284 | 4 | 每圈燃油估算 | Fuel per lap estimate |
| `rain_lights` | `i32` | 1 | 1288 | 4 | 雨灯状态 | Rain lights state |
| `flashing_lights` | `i32` | 1 | 1292 | 4 | 闪灯状态 | Flashing lights state |
| `lights_stage` | `i32` | 1 | 1296 | 4 | 车灯档位 | Lights stage |
| `exhaust_temperature` | `f32` | 1 | 1300 | 4 | 排气温度 | Exhaust temperature |
| `wiper_lv` | `i32` | 1 | 1304 | 4 | 雨刷档位 | Wiper level |
| `driver_stint_total_time_left` | `i32` | 1 | 1308 | 4 | 驾驶 stint 总剩余时间 | Driver stint total time left |
| `driver_stint_time_left` | `i32` | 1 | 1312 | 4 | 驾驶 stint 剩余时间 | Driver stint time left |
| `rain_tyres` | `i32` | 1 | 1316 | 4 | 是否使用雨胎 | Rain tyres state |
| `session_index` | `i32` | 1 | 1320 | 4 | Session 索引 | Session index |
| `used_fuel` | `f32` | 1 | 1324 | 4 | 已用燃油 | Used fuel |
| `delta_lap_time` | `u16` | 15 | 1328 | 30 | 圈速差字符串 | Delta lap time string |
| `i_delta_lap_time` | `i32` | 1 | 1360 | 4 | 圈速差毫秒 | Delta lap time in milliseconds |
| `estimated_lap_time` | `u16` | 15 | 1364 | 30 | 预计圈速字符串 | Estimated lap time string |
| `i_estimated_lap_time` | `i32` | 1 | 1396 | 4 | 预计圈速毫秒 | Estimated lap time in milliseconds |
| `is_delta_positive` | `i32` | 1 | 1400 | 4 | 圈速差是否为正 | Delta positive state |
| `i_split` | `i32` | 1 | 1404 | 4 | 分段时间毫秒 | Split time in milliseconds |
| `is_valid_lap` | `i32` | 1 | 1408 | 4 | 当前圈是否有效；过线前一帧代表刚结束圈的有效性 | Current lap valid state; value before lap reset describes the lap that just ended |
| `fuel_estimated_laps` | `f32` | 1 | 1412 | 4 | 燃油预计可跑圈数 | Estimated laps possible with current fuel level |
| `track_status` | `u16` | 33 | 1416 | 66 | 赛道状态字符串 | Track status string |
| `missing_mandatory_pits` | `i32` | 1 | 1484 | 4 | 剩余强制进站次数 | Missing mandatory pit stops |
| `clock` | `f32` | 1 | 1488 | 4 | 游戏时钟 | Game clock |
| `direction_lights_left` | `i32` | 1 | 1492 | 4 | 左转向灯状态 | Left indicator state |
| `direction_lights_right` | `i32` | 1 | 1496 | 4 | 右转向灯状态 | Right indicator state |
| `global_yellow` | `i32` | 1 | 1500 | 4 | 全局黄旗 | Global yellow flag |
| `global_yellow1` | `i32` | 1 | 1504 | 4 | 黄旗分区 1 | Yellow flag sector 1 |
| `global_yellow2` | `i32` | 1 | 1508 | 4 | 黄旗分区 2 | Yellow flag sector 2 |
| `global_yellow3` | `i32` | 1 | 1512 | 4 | 黄旗分区 3 | Yellow flag sector 3 |
| `global_white` | `i32` | 1 | 1516 | 4 | 全局白旗 | Global white flag |
| `global_green` | `i32` | 1 | 1520 | 4 | 全局绿旗 | Global green flag |
| `global_chequered` | `i32` | 1 | 1524 | 4 | 全局方格旗 | Global chequered flag |
| `global_red` | `i32` | 1 | 1528 | 4 | 全局红旗 | Global red flag |
| `mfd_tyre_set` | `i32` | 1 | 1532 | 4 | MFD 轮胎套装 | MFD tyre set |
| `mfd_fuel_to_add` | `f32` | 1 | 1536 | 4 | MFD 计划加油量 | MFD fuel to add |
| `mfd_tyre_pressure_lf` | `f32` | 1 | 1540 | 4 | MFD 左前目标胎压 | MFD left-front tyre pressure |
| `mfd_tyre_pressure_rf` | `f32` | 1 | 1544 | 4 | MFD 右前目标胎压 | MFD right-front tyre pressure |
| `mfd_tyre_pressure_lr` | `f32` | 1 | 1548 | 4 | MFD 左后目标胎压 | MFD left-rear tyre pressure |
| `mfd_tyre_pressure_rr` | `f32` | 1 | 1552 | 4 | MFD 右后目标胎压 | MFD right-rear tyre pressure |
| `track_grip_status` | `i32` | 1 | 1556 | 4 | 赛道抓地力状态 | Track grip status |
| `rain_intensity` | `i32` | 1 | 1560 | 4 | 当前雨量 | Current rain intensity |
| `rain_intensity_in_10min` | `i32` | 1 | 1564 | 4 | 10 分钟后雨量 | Rain intensity in 10 minutes |
| `rain_intensity_in_30min` | `i32` | 1 | 1568 | 4 | 30 分钟后雨量 | Rain intensity in 30 minutes |
| `current_tyre_set` | `i32` | 1 | 1572 | 4 | 当前轮胎套装 | Current tyre set |
| `strategy_tyre_set` | `i32` | 1 | 1576 | 4 | 策略轮胎套装 | Strategy tyre set |
| `gap_ahead` | `i32` | 1 | 1580 | 4 | 前车差距 | Gap to car ahead |
| `gap_behind` | `i32` | 1 | 1584 | 4 | 后车差距 | Gap to car behind |

## Static Page: `rawStaticPage`

The current implementation captures the static identity prefix used for file metadata and later decoding context.

| Field | Type | Count | Offset | Length | 中文含义 | English meaning |
|---|---|---|---:|---:|---:|---|---|
| `sm_version` | `u16` | 15 | 0 | 30 | 共享内存版本字符串 | Shared memory version string |
| `ac_version` | `u16` | 15 | 30 | 30 | Assetto Corsa/ACC 版本字符串 | Assetto Corsa/ACC version string |
| `number_of_sessions` | `i32` | 1 | 60 | 4 | Session 数量 | Number of sessions |
| `num_cars` | `i32` | 1 | 64 | 4 | 车辆数量 | Number of cars |
| `car_model` | `u16` | 33 | 68 | 66 | 车辆型号字符串 | Car model string |
| `track` | `u16` | 33 | 134 | 66 | 赛道名称字符串 | Track name string |
| `player_name` | `u16` | 33 | 200 | 66 | 玩家名 | Player name |
| `player_surname` | `u16` | 33 | 266 | 66 | 玩家姓 | Player surname |
| `player_nick` | `u16` | 33 | 332 | 66 | 玩家昵称 | Player nickname |
| `sector_count` | `i32` | 1 | 398 | 4 | 赛道分段数 | Number of sectors |
| `max_torque` | `f32` | 1 | 402 | 4 | 最大扭矩 | Maximum torque |
| `max_power` | `f32` | 1 | 406 | 4 | 最大功率 | Maximum power |
| `max_rpm` | `i32` | 1 | 410 | 4 | 最大转速 | Maximum RPM |
| `max_fuel` | `f32` | 1 | 414 | 4 | 最大燃油量 | Maximum fuel |
| `suspension_max_travel` | `f32` | 4 | 418 | 16 | 悬挂最大行程 | Maximum suspension travel |
| `tyre_radius` | `f32` | 4 | 434 | 16 | 轮胎半径 | Tyre radius |
| `max_turbo_boost` | `f32` | 1 | 450 | 4 | 最大涡轮增压 | Maximum turbo boost |
| `deprecated_1` | `f32` | 1 | 454 | 4 | 废弃字段 | Deprecated field |
| `deprecated_2` | `f32` | 1 | 458 | 4 | 废弃字段 | Deprecated field |
| `penalties_enabled` | `i32` | 1 | 462 | 4 | 处罚是否启用 | Penalties enabled |
| `aid_fuel_rate` | `f32` | 1 | 466 | 4 | 燃油消耗辅助 | Fuel rate aid |
| `aid_tire_rate` | `f32` | 1 | 470 | 4 | 轮胎磨损辅助 | Tyre rate aid |
| `aid_mechanical_damage` | `f32` | 1 | 474 | 4 | 机械损伤辅助 | Mechanical damage aid |
| `aid_allow_tyre_blankets` | `i32` | 1 | 478 | 4 | 轮胎毯辅助 | Tyre blankets aid |
| `aid_stability` | `f32` | 1 | 482 | 4 | 稳定性辅助 | Stability aid |
| `aid_auto_clutch` | `i32` | 1 | 486 | 4 | 自动离合器 | Auto clutch |
| `aid_auto_blip` | `i32` | 1 | 490 | 4 | 自动补油 | Auto blip |
| `has_drs` | `i32` | 1 | 494 | 4 | 是否有 DRS | Has DRS |
| `has_ers` | `i32` | 1 | 498 | 4 | 是否有 ERS | Has ERS |
| `has_kers` | `i32` | 1 | 502 | 4 | 是否有 KERS | Has KERS |
| `kers_max_j` | `f32` | 1 | 506 | 4 | KERS 最大能量 J | KERS max energy J |
| `engine_brake_settings_count` | `i32` | 1 | 510 | 4 | 发动机制动设置数 | Engine brake settings count |
| `ers_power_controller_count` | `i32` | 1 | 514 | 4 | ERS 功率控制数 | ERS power controller count |
| `track_spline_length` | `f32` | 1 | 518 | 4 | 赛道样条长度 | Track spline length |
| `track_configuration` | `u16` | 33 | 522 | 66 | 赛道配置字符串 | Track configuration |
| `ers_max_j` | `f32` | 1 | 588 | 4 | ERS 最大能量 | ERS max joule |
| `is_timed_race` | `i32` | 1 | 592 | 4 | 是否限时赛 | Is timed race |
| `has_extra_lap` | `i32` | 1 | 596 | 4 | 是否有额外圈 | Has extra lap |
| `car_skin` | `u16` | 33 | 600 | 66 | 车辆皮肤 | Car skin |
| `reversed_grid_positions` | `i32` | 1 | 666 | 4 | 逆序发车位 | Reversed grid positions |
| `pit_window_start` | `i32` | 1 | 670 | 4 | 进站窗口开始 | Pit window start |
| `pit_window_end` | `i32` | 1 | 674 | 4 | 进站窗口结束 | Pit window end |
| `is_online` | `i32` | 1 | 678 | 4 | 是否在线 | Is online |
| `dry_tyres_name` | `u16` | 33 | 682 | 66 | 干胎名称 | Dry tyres name |
| `wet_tyres_name` | `u16` | 33 | 748 | 66 | 雨胎名称 | Wet tyres name |
