//! Raw item 目录 — 定义所有 raw item 的内部名称、中文描述和单位。
//!
//! 内部匹配使用字符串名称（如 `controls.speed_kmh`），不存在数字 ID。
//! 此模块提供面向用户的 item 列表 API，对外暴露 `{类型}:{路径}` 格式的名称。

use crate::item_key::{ItemKey, ItemType};

/// Raw item 条目
#[derive(Debug, Clone)]
pub struct RawItemEntry {
    /// 完整标识键，如 `raw:controls.speed_kmh`
    pub key: ItemKey,
    /// 中文描述
    pub description: String,
    /// 单位（如 "km/h"、"rpm"、"°C"），可为空
    pub unit: Option<&'static str>,
}

impl RawItemEntry {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        unit: Option<&'static str>,
    ) -> Self {
        RawItemEntry {
            key: ItemKey::new(ItemType::Raw, name),
            description: description.into(),
            unit,
        }
    }
}

/// 返回所有 raw item 的完整目录（含中文描述）
pub fn all_raw_items() -> Vec<RawItemEntry> {
    let mut items = Vec::new();

    // ---- 顶层字段 ----
    items.push(RawItemEntry::new("sample_tick", "采样序号", None));
    items.push(RawItemEntry::new("timestamp_ns", "时间戳（纳秒）", Some("ns")));

    // ---- Controls（车辆操控）----
    let controls: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("physics_packet_id", "物理数据包 ID", None),
        ("graphics_packet_id", "图形数据包 ID", None),
        ("speed_kmh", "车速", Some("km/h")),
        ("gas", "油门开度（0.0~1.0）", None),
        ("brake", "刹车开度（0.0~1.0）", None),
        ("clutch", "离合器开度（0.0~1.0）", None),
        ("steer_angle", "方向盘转角", Some("rad")),
        ("gear", "当前档位（0=空档，-1=倒档）", None),
        ("rpms", "发动机转速", Some("rpm")),
        ("fuel", "剩余燃油", Some("L")),
    ];
    for (name, desc, unit) in controls {
        items.push(RawItemEntry::new(
            format!("controls.{}", name), *desc, *unit,
        ));
    }

    // ---- Motion（运动数据）----
    let motion_scalars: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("heading", "航向角", Some("rad")),
        ("pitch", "俯仰角", Some("rad")),
        ("roll", "侧倾角", Some("rad")),
    ];
    for (name, desc, unit) in motion_scalars {
        items.push(RawItemEntry::new(
            format!("motion.{}", name), *desc, *unit,
        ));
    }
    // 速度数组
    for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
        items.push(RawItemEntry::new(
            format!("motion.velocity[{}]", i),
            format!("世界坐标系速度{}轴分量", axis), Some("m/s"),
        ));
        items.push(RawItemEntry::new(
            format!("motion.acc_g[{}]", i),
            format!("加速度{}轴分量（G力）", axis), Some("G"),
        ));
        items.push(RawItemEntry::new(
            format!("motion.local_velocity[{}]", i),
            format!("车辆局部坐标系速度{}轴分量", axis), Some("m/s"),
        ));
        items.push(RawItemEntry::new(
            format!("motion.local_angular_vel[{}]", i),
            format!("车辆局部坐标系角速度{}轴分量", axis), Some("rad/s"),
        ));
    }

    // ---- Tyres（轮胎数据）----
    let tyres_scalars: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("number_of_tyres_out", "出界轮胎数量", None),
        ("front_brake_compound", "前刹车片材质类型", None),
        ("rear_brake_compound", "后刹车片材质类型", None),
    ];
    for (name, desc, unit) in tyres_scalars {
        items.push(RawItemEntry::new(
            format!("tyres.{}", name), *desc, *unit,
        ));
    }
    // 轮胎数组字段（FL/FR/RL/RR = 前左/前右/后左/后右）
    let tyre_corners: &[&str] = &["前左", "前右", "后左", "后右"];
    let tyre_arrays: &[(&str, &str, Option<&str>)] = &[
        ("wheel_slip", "轮胎滑动率", None),
        ("wheel_load", "轮胎载荷", Some("N")),
        ("wheels_pressure", "胎压", Some("psi")),
        ("wheel_angular_speed", "车轮角速度", Some("rad/s")),
        ("tyre_wear", "轮胎磨损", None),
        ("tyre_dirty_level", "轮胎脏污程度", None),
        ("tyre_core_temperature", "轮胎核心温度", Some("°C")),
        ("camber_rad", "轮胎倾角", Some("rad")),
        ("suspension_travel", "悬挂行程", Some("mm")),
        ("slip_ratio", "滑移率", None),
        ("slip_angle", "滑移角", Some("rad")),
        ("tyre_temp_i", "轮胎内侧温度", Some("°C")),
        ("tyre_temp_m", "轮胎中间温度", Some("°C")),
        ("tyre_temp_o", "轮胎外侧温度", Some("°C")),
        ("tyre_temp", "轮胎表面温度", Some("°C")),
        ("mz", "回正力矩", Some("Nm")),
        ("fx", "纵向力", Some("N")),
        ("fy", "横向力", Some("N")),
        ("suspension_damage", "悬挂损伤", None),
        ("brake_temp", "刹车温度", Some("°C")),
        ("brake_pressure", "刹车压力", Some("bar")),
        ("pad_life", "刹车片寿命", None),
        ("disc_life", "刹车盘寿命", None),
    ];
    for (base_name, desc, unit) in tyre_arrays {
        for (i, corner) in tyre_corners.iter().enumerate() {
            items.push(RawItemEntry::new(
                format!("tyres.{}[{}]", base_name, i),
                format!("{}（{}）", desc, corner),
                *unit,
            ));
        }
    }
    // Tyre 接触点数据（12 个元素 = 4 轮 x 3 轴）
    let contact12_labels: &[&str] = &[
        "前左X", "前左Y", "前左Z", "前右X", "前右Y", "前右Z",
        "后左X", "后左Y", "后左Z", "后右X", "后右Y", "后右Z",
    ];
    for (base_name, desc) in &[
        ("tyre_contact_point", "轮胎接地点坐标"),
        ("tyre_contact_normal", "轮胎接地面法向量"),
        ("tyre_contact_heading", "轮胎接地朝向"),
    ] {
        for (i, label) in contact12_labels.iter().enumerate() {
            items.push(RawItemEntry::new(
                format!("tyres.{}[{}]", base_name, i),
                format!("{}（{}）", desc, label),
                None,
            ));
        }
    }

    // ---- Powertrain（动力总成）----
    let powertrain: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("turbo_boost", "涡轮增压值", Some("bar")),
        ("ballast", "配重", Some("kg")),
        ("kers_charge", "KERS 充电百分比", Some("%")),
        ("kers_input", "KERS 输入功率", Some("kW")),
        ("kers_current_kj", "KERS 当前能量", Some("kJ")),
        ("drs", "DRS 状态（0=关闭，1=开启）", None),
        ("tc", "牵引力控制等级", None),
        ("abs", "ABS 等级", None),
        ("engine_brake", "发动机制动等级", None),
        ("ers_recovery_level", "ERS 回收等级", None),
        ("ers_power_level", "ERS 输出等级", None),
        ("ers_heat_charging", "ERS 热量充电状态", None),
        ("ers_is_charging", "ERS 是否充电中（0/1）", None),
        ("drs_available", "DRS 是否可用（0/1）", None),
        ("drs_enabled", "DRS 是否启用（0/1）", None),
        ("tc_in_action", "TC 是否介入（0/1）", None),
        ("abs_in_action", "ABS 是否介入（0/1）", None),
        ("auto_shifter_on", "自动换挡是否开启（0/1）", None),
        ("current_max_rpm", "当前最大转速", Some("rpm")),
        ("p2p_activations", "P2P 激活次数", None),
        ("p2p_status", "P2P 状态", None),
        ("water_temp", "水温", Some("°C")),
    ];
    for (name, desc, unit) in powertrain {
        items.push(RawItemEntry::new(
            format!("powertrain.{}", name), *desc, *unit,
        ));
    }

    // ---- Session（比赛会话）----
    let session: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("status", "游戏状态（0=Off，1=Replay，2=Live，3=Pause）", None),
        ("session", "会话状态", None),
        ("session_index", "会话序号", None),
        ("completed_laps", "已完成圈数", None),
        ("position", "当前排名", None),
        ("session_time_left", "会话剩余时间", Some("s")),
        ("number_of_laps", "总圈数", None),
        ("current_sector_index", "当前扇区索引", None),
        ("normalized_car_position", "标准化赛道位置（0.0~1.0）", None),
        ("is_in_pit", "是否在维修区（0/1）", None),
        ("is_in_pit_lane", "是否在维修通道（0/1）", None),
        ("mandatory_pit_done", "强制进站是否完成（0/1）", None),
        ("missing_mandatory_pits", "未完成的强制进站次数", None),
        ("penalty_time", "罚时", Some("s")),
        ("penalty_type", "罚时类型", None),
        ("clock", "比赛时钟", Some("s")),
        ("replay_time_multiplier", "回放速度倍率", Some("x")),
        ("is_valid_lap", "当前圈是否有效（0/1）", None),
        ("global_yellow", "全场黄旗（0/1）", None),
        ("global_yellow1", "1号黄旗区（0/1）", None),
        ("global_yellow2", "2号黄旗区（0/1）", None),
        ("global_yellow3", "3号黄旗区（0/1）", None),
        ("global_white", "全场白旗（0/1）", None),
        ("global_green", "全场绿旗（0/1）", None),
        ("global_chequered", "格子旗（0/1）", None),
        ("global_red", "全场红旗（0/1）", None),
        ("gap_ahead_or_tail_value", "与前车差距（ms）或队列位置", Some("ms")),
        ("flag", "当前旗语类型", None),
        ("gap_behind", "与后车差距", Some("ms")),
    ];
    for (name, desc, unit) in session {
        items.push(RawItemEntry::new(
            format!("session.{}", name), *desc, *unit,
        ));
    }

    // ---- Timing（计时数据）----
    let timing: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("i_current_time", "当前圈用时", Some("ms")),
        ("i_last_time", "上一圈用时", Some("ms")),
        ("i_best_time", "最佳圈用时", Some("ms")),
        ("i_split", "分段计时", Some("ms")),
        ("last_sector_time", "上一扇区用时", Some("ms")),
        ("i_delta_lap_time", "与最佳圈时间差", Some("ms")),
        ("is_delta_positive", "时间差是否为正（慢于最佳圈=1）", None),
        ("i_estimated_lap_time", "预估圈时", Some("ms")),
        ("fuel_estimated_laps", "预估可跑圈数", None),
        ("fuel_x_lap", "每圈油耗", Some("L")),
        ("used_fuel", "已消耗燃油", Some("L")),
        ("distance_traveled", "已行驶距离", Some("m")),
        ("observed_slot_before_i_split", "分段前观察槽位", None),
    ];
    for (name, desc, unit) in timing {
        items.push(RawItemEntry::new(
            format!("timing.{}", name), *desc, *unit,
        ));
    }

    // ---- CarState（车辆状态）----
    let car_state_scalars: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("pit_limiter_on", "维修区限速器是否开启（0/1）", None),
        ("ignition_on", "点火是否开启（0/1）", None),
        ("starter_engine_on", "启动马达是否工作（0/1）", None),
        ("is_engine_running", "发动机是否运转（0/1）", None),
        ("is_ai_controlled", "是否AI控制（0/1）", None),
        ("cg_height", "重心高度", Some("mm")),
        ("brake_bias", "刹车平衡", Some("%")),
        ("rain_lights", "雨灯状态", None),
        ("flashing_lights", "闪烁灯状态", None),
        ("lights_stage", "灯光档位", None),
        ("wiper_lv", "雨刮档位", None),
        ("driver_stint_total_time_left", "车手剩余总驾驶时间", Some("s")),
        ("driver_stint_time_left", "车手当前阶段剩余时间", Some("s")),
        ("rain_tyres", "是否使用雨胎（0/1）", None),
        ("current_tyre_set", "当前轮胎组编号", None),
        ("strategy_tyre_set", "策略轮胎组编号", None),
        ("track_grip_status", "赛道抓地力状态", None),
        ("mfd_tyre_set", "MFD显示轮胎组", None),
        ("mfd_fuel_to_add", "MFD显示需加油量", Some("L")),
        ("ideal_line_on", "理想路线辅助是否开启（0/1）", None),
        ("is_setup_menu_visible", "调校菜单是否显示（0/1）", None),
        ("main_display_index", "主显示页面索引", None),
        ("secondary_display_index", "副显示页面索引", None),
        ("direction_lights_left", "左转向灯（0/1）", None),
        ("direction_lights_right", "右转向灯（0/1）", None),
        ("tc_level", "TC 等级", None),
        ("tc_cut", "TC 切断等级", None),
        ("engine_map", "发动机模式", None),
        ("abs_level", "ABS 等级", None),
        ("exhaust_temperature", "排气温度", Some("°C")),
        ("final_ff", "最终前向力反馈", None),
        ("performance_meter", "性能表指数", None),
        ("kerb_vibration", "路肩震动", None),
        ("slip_vibrations", "滑动震动", None),
        ("g_vibrations", "G力震动", None),
        ("abs_vibrations", "ABS震动", None),
    ];
    for (name, desc, unit) in car_state_scalars {
        items.push(RawItemEntry::new(
            format!("car_state.{}", name), *desc, *unit,
        ));
    }
    // 车辆损伤数组（前/后/左/右/中）
    let damage_labels: &[&str] = &["前部", "后部", "左侧", "右侧", "中部"];
    for (i, label) in damage_labels.iter().enumerate() {
        items.push(RawItemEntry::new(
            format!("car_state.car_damage[{}]", i),
            format!("车辆损伤（{}）", label), None,
        ));
    }
    // 底盘高度数组（前/后）
    let ride_labels: &[&str] = &["前轴", "后轴"];
    for (i, label) in ride_labels.iter().enumerate() {
        items.push(RawItemEntry::new(
            format!("car_state.ride_height[{}]", i),
            format!("底盘高度（{}）", label), Some("mm"),
        ));
    }
    // MFD胎压数组（FL/FR/RL/RR）
    for (i, corner) in tyre_corners.iter().enumerate() {
        items.push(RawItemEntry::new(
            format!("car_state.mfd_tyre_pressure[{}]", i),
            format!("MFD显示胎压（{}）", corner), Some("psi"),
        ));
    }

    // ---- Environment（环境数据）----
    let environment: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("air_density", "空气密度", Some("kg/m³")),
        ("air_temp", "气温", Some("°C")),
        ("road_temp", "赛道温度", Some("°C")),
        ("wind_speed", "风速", Some("m/s")),
        ("wind_direction", "风向", Some("rad")),
        ("surface_grip", "路面抓地力系数", None),
        ("rain_intensity", "降雨强度", None),
        ("rain_intensity_in_10min", "未来10分钟降雨强度", None),
        ("rain_intensity_in_30min", "未来30分钟降雨强度", None),
    ];
    for (name, desc, unit) in environment {
        items.push(RawItemEntry::new(
            format!("environment.{}", name), *desc, *unit,
        ));
    }

    // ---- OtherCars（其他车辆）----
    let other_cars: &[(&str, &str, Option<&str>)] = &[
        ("sample_tick", "采样序号", None),
        ("timestamp_ns", "时间戳（纳秒）", Some("ns")),
        ("active_cars", "活跃车辆数", None),
        ("player_car_id", "玩家车辆 ID", None),
    ];
    for (name, desc, unit) in other_cars {
        items.push(RawItemEntry::new(
            format!("other_cars.{}", name), *desc, *unit,
        ));
    }

    items
}
