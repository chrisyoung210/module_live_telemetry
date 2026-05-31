// Re-export cluster IDs from format
pub use crate::format::{
    CLUSTER_CAR_STATE, CLUSTER_CONTROLS, CLUSTER_ENVIRONMENT, CLUSTER_MOTION,
    CLUSTER_OTHER_CARS, CLUSTER_POWERTRAIN, CLUSTER_RAW_PAGES, CLUSTER_SESSION, CLUSTER_TIMING,
    CLUSTER_TYRES,
};

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

// ---------------------------------------------------------------------------
// Recording summary
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct RecordingSummary {
    pub total_samples: u64,
    pub chunk_count: u32,
    pub total_bytes: u64,
    pub footer_offset: u64,
}

// ---------------------------------------------------------------------------
// Legacy raw-page types (kept for old-format import)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct RawPageSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub physics_page: Vec<u8>,
    pub graphics_page: Vec<u8>,
    pub static_page: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct RawGraphicsSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub status: i32,
    pub session: i32,
    pub completed_laps: i32,
    pub current_lap_time_ms: i32,
    pub last_lap_time_ms: i32,
    pub best_lap_time_ms: i32,
    pub distance_traveled_m: f32,
    pub normalized_car_position: f32,
    pub is_valid_lap: i32,
    pub current_sector: i32,
    pub last_sector_time_ms: i32,
    pub in_pit: i32,
    pub in_pit_lane: i32,
    pub delta_lap_time_ms: i32,
}

#[derive(Debug, Clone)]
pub struct RawGraphicsPageSample {
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub page: Vec<u8>,
}

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
    fn default() -> Self { Self { sample_tick: 0, timestamp_ns: 0, status: 0, session: 0, session_index: 0, completed_laps: 0, position: 0, session_time_left: 0.0, number_of_laps: 0, current_sector_index: 0, normalized_car_position: 0.0, is_in_pit: 0, is_in_pit_lane: 0, mandatory_pit_done: 0, missing_mandatory_pits: 0, penalty_time: 0.0, penalty_type: 0, track_status: [0u16; 33], clock: 0.0, replay_time_multiplier: 0.0, is_valid_lap: 0, global_yellow: 0, global_yellow1: 0, global_yellow2: 0, global_yellow3: 0, global_white: 0, global_green: 0, global_chequered: 0, global_red: 0, gap_ahead_or_tail_value: 0 } }
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
