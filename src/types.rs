// Re-export cluster IDs from format
pub use crate::format::{
    CLUSTER_CAR_STATE, CLUSTER_CONTROLS, CLUSTER_ENVIRONMENT, CLUSTER_MOTION,
    CLUSTER_OTHER_CARS, CLUSTER_POWERTRAIN, CLUSTER_SESSION, CLUSTER_TIMING,
    CLUSTER_TYRES,
};

// Helper: parse `field[index]` and return array element as f64
fn try_array_index<T: Copy>(arr: &[T], field: &str, base_name: &str) -> Option<f64>
where f64: From<T>
{
    let prefix = format!("{}[", base_name);
    if field.starts_with(&prefix) && field.ends_with(']') {
        let num_str = &field[prefix.len()..field.len()-1];
        if let Ok(idx) = num_str.parse::<usize>() {
            return arr.get(idx).map(|v| f64::from(*v));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Session metadata (extended with static-page info)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub track_name: String,
    pub car_model: String,
    pub created_unix_ns: u64,
    pub poll_hz: f64,
    pub chunk_rows: usize,
    // static-page information
    pub sm_version: String,
    pub ac_version: String,
    pub number_of_sessions: i32,
    pub num_cars: i32,
    // v3 extended static fields
    pub sector_count: i32,
    pub max_rpm: i32,
    pub max_torque: f32,
    pub max_power: f32,
    pub max_fuel: f32,
    pub penalties_enabled: i32,
    /// Full raw static page bytes (v4); empty for old files
    pub raw_static_bytes: Vec<u8>,
    /// Session type from physics page (v5, 0-8). None for old files.
    pub session_type: Option<i32>,
}

impl SessionMetadata {
    pub fn new(track_name: impl Into<String>, car_model: impl Into<String>, poll_hz: f64) -> Self {
        Self {
            track_name: track_name.into(),
            car_model: car_model.into(),
            created_unix_ns: unix_time_ns(),
            poll_hz,
            chunk_rows: 1024,
            sm_version: String::new(),
            ac_version: String::new(),
            number_of_sessions: 0,
            num_cars: 0,
            sector_count: 0,
            max_rpm: 0,
            max_torque: 0.0,
            max_power: 0.0,
            max_fuel: 0.0,
            penalties_enabled: 0,
            raw_static_bytes: Vec::new(),
            session_type: None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn unix_time_ns() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
fn unix_time_ns() -> u64 {
    0
}

// ---------------------------------------------------------------------------
// Controls (0x0100)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
pub struct ControlSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub physics_packet_id: i32,
    pub graphics_packet_id: i32,
    pub speed_kmh: f32,
    pub gas: f32,
    pub brake: f32,
    pub clutch: f32,
    pub steer_angle: f32,
    pub gear: i32,
    pub rpms: i32,
    pub fuel: f32,
}

impl ControlSample {
    pub fn csv_header() -> &'static str {
        "sampleTick,timestampNs,physicsPacketId,graphicsPacketId,speedKmh,gas,brake,clutch,steerAngle,gear,rpms,fuel"
    }

    pub fn to_csv_row(self) -> String {
        format!(
            "{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{:.6}",
            self.sample_tick,
            self.timestamp_ns,
            self.physics_packet_id,
            self.graphics_packet_id,
            self.speed_kmh,
            self.gas,
            self.brake,
            self.clutch,
            self.steer_angle,
            self.gear,
            self.rpms,
            self.fuel
        )
    }

    /// 按字段名读取 raw 值（用于 raw:controls.xxx 解析）
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "physics_packet_id" => Some(self.physics_packet_id as f64),
            "graphics_packet_id" => Some(self.graphics_packet_id as f64),
            "speed_kmh" => Some(self.speed_kmh as f64),
            "gas" => Some(self.gas as f64),
            "brake" => Some(self.brake as f64),
            "clutch" => Some(self.clutch as f64),
            "steer_angle" => Some(self.steer_angle as f64),
            "gear" => Some(self.gear as f64),
            "rpms" => Some(self.rpms as f64),
            "fuel" => Some(self.fuel as f64),
            _ => None,
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &[
            "sample_tick", "timestamp_ns", "physics_packet_id", "graphics_packet_id",
            "speed_kmh", "gas", "brake", "clutch", "steer_angle", "gear", "rpms", "fuel",
        ]
    }
}

// ---------------------------------------------------------------------------
// Motion (0x0200)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
pub struct MotionSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub velocity: [f32; 3],
    pub acc_g: [f32; 3],
    pub local_velocity: [f32; 3],
    pub local_angular_vel: [f32; 3],
    pub heading: f32,
    pub pitch: f32,
    pub roll: f32,
}

impl MotionSample {
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "heading" => Some(self.heading as f64),
            "pitch" => Some(self.pitch as f64),
            "roll" => Some(self.roll as f64),
            _ => try_array_index(&self.velocity, field, "velocity")
                .or_else(|| try_array_index(&self.acc_g, field, "acc_g"))
                .or_else(|| try_array_index(&self.local_velocity, field, "local_velocity"))
                .or_else(|| try_array_index(&self.local_angular_vel, field, "local_angular_vel")),
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &[
            "sample_tick", "timestamp_ns",
            "velocity[0]", "velocity[1]", "velocity[2]",
            "acc_g[0]", "acc_g[1]", "acc_g[2]",
            "local_velocity[0]", "local_velocity[1]", "local_velocity[2]",
            "local_angular_vel[0]", "local_angular_vel[1]", "local_angular_vel[2]",
            "heading", "pitch", "roll",
        ]
    }
}

// ---------------------------------------------------------------------------
// Tyres (0x0300)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct TyreSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub wheel_slip: [f32; 4],
    pub wheel_load: [f32; 4],
    pub wheels_pressure: [f32; 4],
    pub wheel_angular_speed: [f32; 4],
    pub tyre_wear: [f32; 4],
    pub tyre_dirty_level: [f32; 4],
    pub tyre_core_temperature: [f32; 4],
    pub camber_rad: [f32; 4],
    pub suspension_travel: [f32; 4],
    pub slip_ratio: [f32; 4],
    pub slip_angle: [f32; 4],
    pub tyre_temp_i: [f32; 4],
    pub tyre_temp_m: [f32; 4],
    pub tyre_temp_o: [f32; 4],
    pub tyre_temp: [f32; 4],
    pub mz: [f32; 4],
    pub fx: [f32; 4],
    pub fy: [f32; 4],
    pub suspension_damage: [f32; 4],
    pub brake_temp: [f32; 4],
    pub brake_pressure: [f32; 4],
    pub pad_life: [f32; 4],
    pub disc_life: [f32; 4],
    pub tyre_contact_point: [f32; 12],
    pub tyre_contact_normal: [f32; 12],
    pub tyre_contact_heading: [f32; 12],
    pub number_of_tyres_out: i32,
    pub front_brake_compound: i32,
    pub rear_brake_compound: i32,
}

impl TyreSample {
    #[allow(clippy::too_many_lines)]
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "number_of_tyres_out" => Some(self.number_of_tyres_out as f64),
            "front_brake_compound" => Some(self.front_brake_compound as f64),
            "rear_brake_compound" => Some(self.rear_brake_compound as f64),
            _ => try_array_index(&self.wheel_slip, field, "wheel_slip")
                .or_else(|| try_array_index(&self.wheel_load, field, "wheel_load"))
                .or_else(|| try_array_index(&self.wheels_pressure, field, "wheels_pressure"))
                .or_else(|| try_array_index(&self.wheel_angular_speed, field, "wheel_angular_speed"))
                .or_else(|| try_array_index(&self.tyre_wear, field, "tyre_wear"))
                .or_else(|| try_array_index(&self.tyre_dirty_level, field, "tyre_dirty_level"))
                .or_else(|| try_array_index(&self.tyre_core_temperature, field, "tyre_core_temperature"))
                .or_else(|| try_array_index(&self.camber_rad, field, "camber_rad"))
                .or_else(|| try_array_index(&self.suspension_travel, field, "suspension_travel"))
                .or_else(|| try_array_index(&self.slip_ratio, field, "slip_ratio"))
                .or_else(|| try_array_index(&self.slip_angle, field, "slip_angle"))
                .or_else(|| try_array_index(&self.tyre_temp_i, field, "tyre_temp_i"))
                .or_else(|| try_array_index(&self.tyre_temp_m, field, "tyre_temp_m"))
                .or_else(|| try_array_index(&self.tyre_temp_o, field, "tyre_temp_o"))
                .or_else(|| try_array_index(&self.tyre_temp, field, "tyre_temp"))
                .or_else(|| try_array_index(&self.mz, field, "mz"))
                .or_else(|| try_array_index(&self.fx, field, "fx"))
                .or_else(|| try_array_index(&self.fy, field, "fy"))
                .or_else(|| try_array_index(&self.suspension_damage, field, "suspension_damage"))
                .or_else(|| try_array_index(&self.brake_temp, field, "brake_temp"))
                .or_else(|| try_array_index(&self.brake_pressure, field, "brake_pressure"))
                .or_else(|| try_array_index(&self.pad_life, field, "pad_life"))
                .or_else(|| try_array_index(&self.disc_life, field, "disc_life"))
                .or_else(|| try_array_index(&self.tyre_contact_point, field, "tyre_contact_point"))
                .or_else(|| try_array_index(&self.tyre_contact_normal, field, "tyre_contact_normal"))
                .or_else(|| try_array_index(&self.tyre_contact_heading, field, "tyre_contact_heading")),
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &[
            "sample_tick", "timestamp_ns",
            "wheel_slip[0]", "wheel_slip[1]", "wheel_slip[2]", "wheel_slip[3]",
            "wheel_load[0]", "wheel_load[1]", "wheel_load[2]", "wheel_load[3]",
            "wheels_pressure[0]", "wheels_pressure[1]", "wheels_pressure[2]", "wheels_pressure[3]",
            "wheel_angular_speed[0]", "wheel_angular_speed[1]", "wheel_angular_speed[2]", "wheel_angular_speed[3]",
            "tyre_wear[0]", "tyre_wear[1]", "tyre_wear[2]", "tyre_wear[3]",
            "tyre_dirty_level[0]", "tyre_dirty_level[1]", "tyre_dirty_level[2]", "tyre_dirty_level[3]",
            "tyre_core_temperature[0]", "tyre_core_temperature[1]", "tyre_core_temperature[2]", "tyre_core_temperature[3]",
            "camber_rad[0]", "camber_rad[1]", "camber_rad[2]", "camber_rad[3]",
            "suspension_travel[0]", "suspension_travel[1]", "suspension_travel[2]", "suspension_travel[3]",
            "slip_ratio[0]", "slip_ratio[1]", "slip_ratio[2]", "slip_ratio[3]",
            "slip_angle[0]", "slip_angle[1]", "slip_angle[2]", "slip_angle[3]",
            "tyre_temp_i[0]", "tyre_temp_i[1]", "tyre_temp_i[2]", "tyre_temp_i[3]",
            "tyre_temp_m[0]", "tyre_temp_m[1]", "tyre_temp_m[2]", "tyre_temp_m[3]",
            "tyre_temp_o[0]", "tyre_temp_o[1]", "tyre_temp_o[2]", "tyre_temp_o[3]",
            "tyre_temp[0]", "tyre_temp[1]", "tyre_temp[2]", "tyre_temp[3]",
            "mz[0]", "mz[1]", "mz[2]", "mz[3]",
            "fx[0]", "fx[1]", "fx[2]", "fx[3]",
            "fy[0]", "fy[1]", "fy[2]", "fy[3]",
            "suspension_damage[0]", "suspension_damage[1]", "suspension_damage[2]", "suspension_damage[3]",
            "brake_temp[0]", "brake_temp[1]", "brake_temp[2]", "brake_temp[3]",
            "brake_pressure[0]", "brake_pressure[1]", "brake_pressure[2]", "brake_pressure[3]",
            "pad_life[0]", "pad_life[1]", "pad_life[2]", "pad_life[3]",
            "disc_life[0]", "disc_life[1]", "disc_life[2]", "disc_life[3]",
            "tyre_contact_point[0]", "tyre_contact_point[1]", "tyre_contact_point[2]",
            "tyre_contact_point[3]", "tyre_contact_point[4]", "tyre_contact_point[5]",
            "tyre_contact_point[6]", "tyre_contact_point[7]", "tyre_contact_point[8]",
            "tyre_contact_point[9]", "tyre_contact_point[10]", "tyre_contact_point[11]",
            "tyre_contact_normal[0]", "tyre_contact_normal[1]", "tyre_contact_normal[2]",
            "tyre_contact_normal[3]", "tyre_contact_normal[4]", "tyre_contact_normal[5]",
            "tyre_contact_normal[6]", "tyre_contact_normal[7]", "tyre_contact_normal[8]",
            "tyre_contact_normal[9]", "tyre_contact_normal[10]", "tyre_contact_normal[11]",
            "tyre_contact_heading[0]", "tyre_contact_heading[1]", "tyre_contact_heading[2]",
            "tyre_contact_heading[3]", "tyre_contact_heading[4]", "tyre_contact_heading[5]",
            "tyre_contact_heading[6]", "tyre_contact_heading[7]", "tyre_contact_heading[8]",
            "tyre_contact_heading[9]", "tyre_contact_heading[10]", "tyre_contact_heading[11]",
            "number_of_tyres_out", "front_brake_compound", "rear_brake_compound",
        ]
    }
}

// ---------------------------------------------------------------------------
// Powertrain (0x0400)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
pub struct PowertrainSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub turbo_boost: f32,
    pub ballast: f32,
    pub kers_charge: f32,
    pub kers_input: f32,
    pub kers_current_kj: f32,
    pub drs: f32,
    pub tc: f32,
    pub abs: f32,
    pub engine_brake: i32,
    pub ers_recovery_level: i32,
    pub ers_power_level: i32,
    pub ers_heat_charging: i32,
    pub ers_is_charging: i32,
    pub drs_available: i32,
    pub drs_enabled: i32,
    pub tc_in_action: i32,
    pub abs_in_action: i32,
    pub auto_shifter_on: i32,
    pub current_max_rpm: i32,
    pub p2p_activations: i32,
    pub p2p_status: i32,
    pub water_temp: f32,
}

impl PowertrainSample {
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "turbo_boost" => Some(self.turbo_boost as f64),
            "ballast" => Some(self.ballast as f64),
            "kers_charge" => Some(self.kers_charge as f64),
            "kers_input" => Some(self.kers_input as f64),
            "kers_current_kj" => Some(self.kers_current_kj as f64),
            "drs" => Some(self.drs as f64),
            "tc" => Some(self.tc as f64),
            "abs" => Some(self.abs as f64),
            "engine_brake" => Some(self.engine_brake as f64),
            "ers_recovery_level" => Some(self.ers_recovery_level as f64),
            "ers_power_level" => Some(self.ers_power_level as f64),
            "ers_heat_charging" => Some(self.ers_heat_charging as f64),
            "ers_is_charging" => Some(self.ers_is_charging as f64),
            "drs_available" => Some(self.drs_available as f64),
            "drs_enabled" => Some(self.drs_enabled as f64),
            "tc_in_action" => Some(self.tc_in_action as f64),
            "abs_in_action" => Some(self.abs_in_action as f64),
            "auto_shifter_on" => Some(self.auto_shifter_on as f64),
            "current_max_rpm" => Some(self.current_max_rpm as f64),
            "p2p_activations" => Some(self.p2p_activations as f64),
            "p2p_status" => Some(self.p2p_status as f64),
            "water_temp" => Some(self.water_temp as f64),
            _ => None,
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &[
            "sample_tick", "timestamp_ns",
            "turbo_boost", "ballast", "kers_charge", "kers_input", "kers_current_kj",
            "drs", "tc", "abs", "engine_brake",
            "ers_recovery_level", "ers_power_level", "ers_heat_charging", "ers_is_charging",
            "drs_available", "drs_enabled", "tc_in_action", "abs_in_action",
            "auto_shifter_on", "current_max_rpm", "p2p_activations", "p2p_status",
            "water_temp",
        ]
    }
}

// ---------------------------------------------------------------------------
// Session (0x0500)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct SessionSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub status: i32,
    pub session: i32,
    pub session_index: i32,
    pub completed_laps: i32,
    pub position: i32,
    pub session_time_left: f32,
    pub number_of_laps: i32,
    pub current_sector_index: i32,
    pub normalized_car_position: f32,
    pub is_in_pit: i32,
    pub is_in_pit_lane: i32,
    pub mandatory_pit_done: i32,
    pub missing_mandatory_pits: i32,
    pub penalty_time: f32,
    pub penalty_type: i32,
    pub track_status: [u16; 33],
    pub clock: f32,
    pub replay_time_multiplier: f32,
    pub is_valid_lap: i32,
    pub global_yellow: i32,
    pub global_yellow1: i32,
    pub global_yellow2: i32,
    pub global_yellow3: i32,
    pub global_white: i32,
    pub global_green: i32,
    pub global_chequered: i32,
    pub global_red: i32,
    pub gap_ahead_or_tail_value: i32,
    pub flag: i32,
    pub gap_behind: i32,
}

impl SessionSample {
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "status" => Some(self.status as f64),
            "session" => Some(self.session as f64),
            "session_index" => Some(self.session_index as f64),
            "completed_laps" => Some(self.completed_laps as f64),
            "position" => Some(self.position as f64),
            "session_time_left" => Some(self.session_time_left as f64),
            "number_of_laps" => Some(self.number_of_laps as f64),
            "current_sector_index" => Some(self.current_sector_index as f64),
            "normalized_car_position" => Some(self.normalized_car_position as f64),
            "is_in_pit" => Some(self.is_in_pit as f64),
            "is_in_pit_lane" => Some(self.is_in_pit_lane as f64),
            "mandatory_pit_done" => Some(self.mandatory_pit_done as f64),
            "missing_mandatory_pits" => Some(self.missing_mandatory_pits as f64),
            "penalty_time" => Some(self.penalty_time as f64),
            "penalty_type" => Some(self.penalty_type as f64),
            "clock" => Some(self.clock as f64),
            "replay_time_multiplier" => Some(self.replay_time_multiplier as f64),
            "is_valid_lap" => Some(self.is_valid_lap as f64),
            "global_yellow" => Some(self.global_yellow as f64),
            "global_yellow1" => Some(self.global_yellow1 as f64),
            "global_yellow2" => Some(self.global_yellow2 as f64),
            "global_yellow3" => Some(self.global_yellow3 as f64),
            "global_white" => Some(self.global_white as f64),
            "global_green" => Some(self.global_green as f64),
            "global_chequered" => Some(self.global_chequered as f64),
            "global_red" => Some(self.global_red as f64),
            "gap_ahead_or_tail_value" => Some(self.gap_ahead_or_tail_value as f64),
            "flag" => Some(self.flag as f64),
            "gap_behind" => Some(self.gap_behind as f64),
            _ => None,
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &[
            "sample_tick", "timestamp_ns",
            "status", "session", "session_index",
            "completed_laps", "position", "session_time_left", "number_of_laps",
            "current_sector_index", "normalized_car_position",
            "is_in_pit", "is_in_pit_lane", "mandatory_pit_done", "missing_mandatory_pits",
            "penalty_time", "penalty_type",
            "clock", "replay_time_multiplier", "is_valid_lap",
            "global_yellow", "global_yellow1", "global_yellow2", "global_yellow3",
            "global_white", "global_green", "global_chequered", "global_red",
            "gap_ahead_or_tail_value", "flag", "gap_behind",
        ]
    }
}

// ---------------------------------------------------------------------------
// Timing (0x0600)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct TimingSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub i_current_time: i32,
    pub i_last_time: i32,
    pub i_best_time: i32,
    pub i_split: i32,
    pub last_sector_time: i32,
    pub i_delta_lap_time: i32,
    pub is_delta_positive: i32,
    pub i_estimated_lap_time: i32,
    pub fuel_estimated_laps: f32,
    pub fuel_x_lap: f32,
    pub used_fuel: f32,
    pub distance_traveled: f32,
    pub current_time_str: [u16; 15],
    pub last_time_str: [u16; 15],
    pub best_time_str: [u16; 15],
    pub split_str: [u16; 15],
    pub delta_lap_time_str: [u16; 15],
    pub estimated_lap_time_str: [u16; 15],
    pub observed_slot_before_i_split: i32,
}

impl TimingSample {
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "i_current_time" => Some(self.i_current_time as f64),
            "i_last_time" => Some(self.i_last_time as f64),
            "i_best_time" => Some(self.i_best_time as f64),
            "i_split" => Some(self.i_split as f64),
            "last_sector_time" => Some(self.last_sector_time as f64),
            "i_delta_lap_time" => Some(self.i_delta_lap_time as f64),
            "is_delta_positive" => Some(self.is_delta_positive as f64),
            "i_estimated_lap_time" => Some(self.i_estimated_lap_time as f64),
            "fuel_estimated_laps" => Some(self.fuel_estimated_laps as f64),
            "fuel_x_lap" => Some(self.fuel_x_lap as f64),
            "used_fuel" => Some(self.used_fuel as f64),
            "distance_traveled" => Some(self.distance_traveled as f64),
            "observed_slot_before_i_split" => Some(self.observed_slot_before_i_split as f64),
            _ => None,
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &[
            "sample_tick", "timestamp_ns",
            "i_current_time", "i_last_time", "i_best_time", "i_split",
            "last_sector_time", "i_delta_lap_time", "is_delta_positive",
            "i_estimated_lap_time", "fuel_estimated_laps", "fuel_x_lap",
            "used_fuel", "distance_traveled", "observed_slot_before_i_split",
        ]
    }
}

// ---------------------------------------------------------------------------
// CarState (0x0700)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct CarStateSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub car_damage: [f32; 5],
    pub pit_limiter_on: i32,
    pub ride_height: [f32; 2],
    pub ignition_on: i32,
    pub starter_engine_on: i32,
    pub is_engine_running: i32,
    pub is_ai_controlled: i32,
    pub cg_height: f32,
    pub brake_bias: f32,
    pub rain_lights: i32,
    pub flashing_lights: i32,
    pub lights_stage: i32,
    pub wiper_lv: i32,
    pub driver_stint_total_time_left: i32,
    pub driver_stint_time_left: i32,
    pub rain_tyres: i32,
    pub current_tyre_set: i32,
    pub strategy_tyre_set: i32,
    pub track_grip_status: i32,
    pub tyre_compound_str: [u16; 33],
    pub mfd_tyre_set: i32,
    pub mfd_fuel_to_add: f32,
    pub mfd_tyre_pressure: [f32; 4],
    pub ideal_line_on: i32,
    pub is_setup_menu_visible: i32,
    pub main_display_index: i32,
    pub secondary_display_index: i32,
    pub direction_lights_left: i32,
    pub direction_lights_right: i32,
    pub tc_level: i32,
    pub tc_cut: i32,
    pub engine_map: i32,
    pub abs_level: i32,
    pub exhaust_temperature: f32,
    pub final_ff: f32,
    pub performance_meter: f32,
    pub kerb_vibration: f32,
    pub slip_vibrations: f32,
    pub g_vibrations: f32,
    pub abs_vibrations: f32,
}

impl CarStateSample {
    #[allow(clippy::too_many_lines)]
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "pit_limiter_on" => Some(self.pit_limiter_on as f64),
            "ignition_on" => Some(self.ignition_on as f64),
            "starter_engine_on" => Some(self.starter_engine_on as f64),
            "is_engine_running" => Some(self.is_engine_running as f64),
            "is_ai_controlled" => Some(self.is_ai_controlled as f64),
            "cg_height" => Some(self.cg_height as f64),
            "brake_bias" => Some(self.brake_bias as f64),
            "rain_lights" => Some(self.rain_lights as f64),
            "flashing_lights" => Some(self.flashing_lights as f64),
            "lights_stage" => Some(self.lights_stage as f64),
            "wiper_lv" => Some(self.wiper_lv as f64),
            "driver_stint_total_time_left" => Some(self.driver_stint_total_time_left as f64),
            "driver_stint_time_left" => Some(self.driver_stint_time_left as f64),
            "rain_tyres" => Some(self.rain_tyres as f64),
            "current_tyre_set" => Some(self.current_tyre_set as f64),
            "strategy_tyre_set" => Some(self.strategy_tyre_set as f64),
            "track_grip_status" => Some(self.track_grip_status as f64),
            "mfd_tyre_set" => Some(self.mfd_tyre_set as f64),
            "mfd_fuel_to_add" => Some(self.mfd_fuel_to_add as f64),
            "ideal_line_on" => Some(self.ideal_line_on as f64),
            "is_setup_menu_visible" => Some(self.is_setup_menu_visible as f64),
            "main_display_index" => Some(self.main_display_index as f64),
            "secondary_display_index" => Some(self.secondary_display_index as f64),
            "direction_lights_left" => Some(self.direction_lights_left as f64),
            "direction_lights_right" => Some(self.direction_lights_right as f64),
            "tc_level" => Some(self.tc_level as f64),
            "tc_cut" => Some(self.tc_cut as f64),
            "engine_map" => Some(self.engine_map as f64),
            "abs_level" => Some(self.abs_level as f64),
            "exhaust_temperature" => Some(self.exhaust_temperature as f64),
            "final_ff" => Some(self.final_ff as f64),
            "performance_meter" => Some(self.performance_meter as f64),
            "kerb_vibration" => Some(self.kerb_vibration as f64),
            "slip_vibrations" => Some(self.slip_vibrations as f64),
            "g_vibrations" => Some(self.g_vibrations as f64),
            "abs_vibrations" => Some(self.abs_vibrations as f64),
            _ => try_array_index(&self.car_damage, field, "car_damage")
                .or_else(|| try_array_index(&self.ride_height, field, "ride_height"))
                .or_else(|| try_array_index(&self.mfd_tyre_pressure, field, "mfd_tyre_pressure")),
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &[
            "sample_tick", "timestamp_ns",
            "car_damage[0]", "car_damage[1]", "car_damage[2]", "car_damage[3]", "car_damage[4]",
            "pit_limiter_on",
            "ride_height[0]", "ride_height[1]",
            "ignition_on", "starter_engine_on", "is_engine_running", "is_ai_controlled",
            "cg_height", "brake_bias", "rain_lights", "flashing_lights", "lights_stage",
            "wiper_lv", "driver_stint_total_time_left", "driver_stint_time_left",
            "rain_tyres", "current_tyre_set", "strategy_tyre_set", "track_grip_status",
            "mfd_tyre_set", "mfd_fuel_to_add",
            "mfd_tyre_pressure[0]", "mfd_tyre_pressure[1]", "mfd_tyre_pressure[2]", "mfd_tyre_pressure[3]",
            "ideal_line_on", "is_setup_menu_visible",
            "main_display_index", "secondary_display_index",
            "direction_lights_left", "direction_lights_right",
            "tc_level", "tc_cut", "engine_map", "abs_level",
            "exhaust_temperature", "final_ff", "performance_meter",
            "kerb_vibration", "slip_vibrations", "g_vibrations", "abs_vibrations",
        ]
    }
}

// ---------------------------------------------------------------------------
// Environment (0x0800)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
pub struct EnvironmentSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub air_density: f32,
    pub air_temp: f32,
    pub road_temp: f32,
    pub wind_speed: f32,
    pub wind_direction: f32,
    pub surface_grip: f32,
    pub rain_intensity: i32,
    pub rain_intensity_in_10min: i32,
    pub rain_intensity_in_30min: i32,
}

impl EnvironmentSample {
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "air_density" => Some(self.air_density as f64),
            "air_temp" => Some(self.air_temp as f64),
            "road_temp" => Some(self.road_temp as f64),
            "wind_speed" => Some(self.wind_speed as f64),
            "wind_direction" => Some(self.wind_direction as f64),
            "surface_grip" => Some(self.surface_grip as f64),
            "rain_intensity" => Some(self.rain_intensity as f64),
            "rain_intensity_in_10min" => Some(self.rain_intensity_in_10min as f64),
            "rain_intensity_in_30min" => Some(self.rain_intensity_in_30min as f64),
            _ => None,
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &[
            "sample_tick", "timestamp_ns",
            "air_density", "air_temp", "road_temp",
            "wind_speed", "wind_direction", "surface_grip",
            "rain_intensity", "rain_intensity_in_10min", "rain_intensity_in_30min",
        ]
    }
}

// ---------------------------------------------------------------------------
// OtherCars (0x0900)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct OtherCarsSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub active_cars: i32,
    pub player_car_id: i32,
    pub car_coordinates: Vec<f32>, // 180 elements
    pub car_id: Vec<i32>,          // 60 elements
}

impl OtherCarsSample {
    pub fn raw_field_value(&self, field: &str) -> Option<f64> {
        match field {
            "sample_tick" => Some(self.sample_tick as f64),
            "timestamp_ns" => Some(self.timestamp_ns as f64),
            "active_cars" => Some(self.active_cars as f64),
            "player_car_id" => Some(self.player_car_id as f64),
            _ => None,
        }
    }

    pub fn raw_field_names() -> &'static [&'static str] {
        &["sample_tick", "timestamp_ns", "active_cars", "player_car_id"]
    }
}

// ---------------------------------------------------------------------------
// Recording summary
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct RecordingSummary {
    pub total_samples: u64,
    pub chunk_count: u32,
    pub total_bytes: u64,
    pub footer_offset: u64,
    pub duration: std::time::Duration,
}

// Lap index entry (stored in file footer after FOOT block)
#[derive(Debug, Clone, Copy)]
pub struct LapIndexEntry {
    pub lap_number: i32,
    pub start_tick: u64,
    pub end_tick: u64,
    pub sample_count: u32,
    pub is_valid: i32,
    pub is_out_lap: i32,
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// ACC session kind
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccSessionKind {
    Practice,
    Qualify,
    Race,
    Hotlap,
    TimeAttack,
    Drift,
    Drag,
    Hotstint,
    HotlapSuperpole,
    Unknown(i32),
}

impl AccSessionKind {
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Practice,
            4 => Self::Qualify,
            9 => Self::Race,
            10 => Self::Hotlap,
            11 => Self::TimeAttack,
            12 => Self::Drift,
            13 => Self::Drag,
            14 => Self::Hotstint,
            15 => Self::HotlapSuperpole,
            other => Self::Unknown(other),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Practice => "practice",
            Self::Qualify => "qualify",
            Self::Race => "race",
            Self::Hotlap => "hotlap",
            Self::TimeAttack => "time_attack",
            Self::Drift => "drift",
            Self::Drag => "drag",
            Self::Hotstint => "hotstint",
            Self::HotlapSuperpole => "hotlap_superpole",
            Self::Unknown(_) => "unknown",
        }
    }
}

// ---- Default implementations ----
impl Default for ControlSample {
    fn default() -> Self {
        Self { sample_tick: 0, timestamp_ns: 0, physics_packet_id: 0, graphics_packet_id: 0, speed_kmh: 0.0, gas: 0.0, brake: 0.0, clutch: 0.0, steer_angle: 0.0, gear: 0, rpms: 0, fuel: 0.0 }
    }
}
impl Default for MotionSample {
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, velocity: [0.0; 3], acc_g: [0.0; 3], local_velocity: [0.0; 3], local_angular_vel: [0.0; 3], heading: 0.0, pitch: 0.0, roll: 0.0 } }
}
impl Default for TyreSample {
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, wheel_slip: [0.0; 4], wheel_load: [0.0; 4], wheels_pressure: [0.0; 4], wheel_angular_speed: [0.0; 4], tyre_wear: [0.0; 4], tyre_dirty_level: [0.0; 4], tyre_core_temperature: [0.0; 4], camber_rad: [0.0; 4], suspension_travel: [0.0; 4], slip_ratio: [0.0; 4], slip_angle: [0.0; 4], tyre_temp_i: [0.0; 4], tyre_temp_m: [0.0; 4], tyre_temp_o: [0.0; 4], tyre_temp: [0.0; 4], mz: [0.0; 4], fx: [0.0; 4], fy: [0.0; 4], suspension_damage: [0.0; 4], brake_temp: [0.0; 4], brake_pressure: [0.0; 4], pad_life: [0.0; 4], disc_life: [0.0; 4], tyre_contact_point: [0.0; 12], tyre_contact_normal: [0.0; 12], tyre_contact_heading: [0.0; 12], number_of_tyres_out: 0, front_brake_compound: 0, rear_brake_compound: 0 } }
}
impl Default for PowertrainSample {
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, turbo_boost: 0.0, ballast: 0.0, kers_charge: 0.0, kers_input: 0.0, kers_current_kj: 0.0, drs: 0.0, tc: 0.0, abs: 0.0, engine_brake: 0, ers_recovery_level: 0, ers_power_level: 0, ers_heat_charging: 0, ers_is_charging: 0, drs_available: 0, drs_enabled: 0, tc_in_action: 0, abs_in_action: 0, auto_shifter_on: 0, current_max_rpm: 0, p2p_activations: 0, p2p_status: 0, water_temp: 0.0 } }
}
impl Default for SessionSample {
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, status: 0, session: 0, session_index: 0, completed_laps: 0, position: 0, session_time_left: 0.0, number_of_laps: 0, current_sector_index: 0, normalized_car_position: 0.0, is_in_pit: 0, is_in_pit_lane: 0, mandatory_pit_done: 0, missing_mandatory_pits: 0, penalty_time: 0.0, penalty_type: 0, track_status: [0u16; 33], clock: 0.0, replay_time_multiplier: 0.0, is_valid_lap: 0, global_yellow: 0, global_yellow1: 0, global_yellow2: 0, global_yellow3: 0, global_white: 0, global_green: 0, global_chequered: 0, global_red: 0, gap_ahead_or_tail_value: 0, flag: 0, gap_behind: 0 } }
}
impl Default for TimingSample {
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, i_current_time: 0, i_last_time: 0, i_best_time: 0, i_split: 0, last_sector_time: 0, i_delta_lap_time: 0, is_delta_positive: 0, i_estimated_lap_time: 0, fuel_estimated_laps: 0.0, fuel_x_lap: 0.0, used_fuel: 0.0, distance_traveled: 0.0, current_time_str: [0u16; 15], last_time_str: [0u16; 15], best_time_str: [0u16; 15], split_str: [0u16; 15], delta_lap_time_str: [0u16; 15], estimated_lap_time_str: [0u16; 15], observed_slot_before_i_split: 0 } }
}
impl Default for CarStateSample {
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, car_damage: [0.0; 5], pit_limiter_on: 0, ride_height: [0.0; 2], ignition_on: 0, starter_engine_on: 0, is_engine_running: 0, is_ai_controlled: 0, cg_height: 0.0, brake_bias: 0.0, rain_lights: 0, flashing_lights: 0, lights_stage: 0, wiper_lv: 0, driver_stint_total_time_left: 0, driver_stint_time_left: 0, rain_tyres: 0, current_tyre_set: 0, strategy_tyre_set: 0, track_grip_status: 0, tyre_compound_str: [0u16; 33], mfd_tyre_set: 0, mfd_fuel_to_add: 0.0, mfd_tyre_pressure: [0.0; 4], ideal_line_on: 0, is_setup_menu_visible: 0, main_display_index: 0, secondary_display_index: 0, direction_lights_left: 0, direction_lights_right: 0, tc_level: 0, tc_cut: 0, engine_map: 0, abs_level: 0, exhaust_temperature: 0.0, final_ff: 0.0, performance_meter: 0.0, kerb_vibration: 0.0, slip_vibrations: 0.0, g_vibrations: 0.0, abs_vibrations: 0.0 } }
}
impl Default for EnvironmentSample {
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, air_density: 0.0, air_temp: 0.0, road_temp: 0.0, wind_speed: 0.0, wind_direction: 0.0, surface_grip: 0.0, rain_intensity: 0, rain_intensity_in_10min: 0, rain_intensity_in_30min: 0 } }
}
impl Default for OtherCarsSample {
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, active_cars: 0, player_car_id: 0, car_coordinates: vec![0.0f32; 180], car_id: vec![0i32; 60] } }
}
