use crate::error::{TelemetryError, TelemetryResult};
use crate::types::{
    CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
    PowertrainSample, SessionSample, TimingSample, TyreSample,
};
use crate::writer::TelemetryFrame;

pub const ACC_STATUS_OFF: i32 = 0;
pub const ACC_STATUS_REPLAY: i32 = 1;
pub const ACC_STATUS_LIVE: i32 = 2;
pub const ACC_STATUS_PAUSE: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccGameStatus {
    Off,
    Replay,
    Live,
    Pause,
    Unknown(i32),
    Unavailable,
}

impl AccGameStatus {
    pub fn from_raw(value: i32) -> Self {
        match value {
            ACC_STATUS_OFF => Self::Off,
            ACC_STATUS_REPLAY => Self::Replay,
            ACC_STATUS_LIVE => Self::Live,
            ACC_STATUS_PAUSE => Self::Pause,
            other => Self::Unknown(other),
        }
    }

    pub fn is_live(self) -> bool {
        self == Self::Live
    }

    pub fn is_pause(self) -> bool {
        self == Self::Pause
    }

    pub fn label(self) -> String {
        match self {
            Self::Off => "off".to_string(),
            Self::Replay => "replay".to_string(),
            Self::Live => "live".to_string(),
            Self::Pause => "pause".to_string(),
            Self::Unknown(value) => format!("unknown({value})"),
            Self::Unavailable => "unavailable".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AccSessionInfo {
    pub track_name: String,
    pub car_model: String,
}

impl Default for AccSessionInfo {
    fn default() -> Self {
        Self {
            track_name: "unknown_track".to_string(),
            car_model: "unknown_car".to_string(),
        }
    }
}

#[cfg(windows)]
pub struct AccSharedMemoryReader {
    physics_mapping: WindowsSharedMemory,
    graphics_mapping: WindowsSharedMemory,
    static_mapping: WindowsSharedMemory,
    last_physics_packet_id: i32,
}

#[cfg(not(windows))]
pub struct AccSharedMemoryReader;

impl AccSharedMemoryReader {
    #[cfg(windows)]
    pub fn open() -> TelemetryResult<Self> {
        Ok(Self {
            physics_mapping: WindowsSharedMemory::open_physics()?,
            graphics_mapping: WindowsSharedMemory::open_graphics()?,
            static_mapping: WindowsSharedMemory::open_static()?,
            last_physics_packet_id: i32::MIN,
        })
    }

    #[cfg(not(windows))]
    pub fn open() -> TelemetryResult<Self> {
        Err(TelemetryError::InvalidArgument(
            "ACC shared memory is only available on Windows".to_string(),
        ))
    }

    #[cfg(windows)]
    pub fn status(&self) -> TelemetryResult<AccGameStatus> {
        Ok(AccGameStatus::from_raw(self.graphics_mapping.status()?))
    }

    #[cfg(not(windows))]
    pub fn status(&self) -> TelemetryResult<AccGameStatus> {
        Ok(AccGameStatus::Unavailable)
    }

    #[cfg(windows)]
    pub fn session_info(&self) -> AccSessionInfo {
        self.static_mapping.static_snapshot().unwrap_or_default()
    }

    #[cfg(not(windows))]
    pub fn session_info(&self) -> AccSessionInfo {
        AccSessionInfo::default()
    }

    #[cfg(windows)]
    pub fn read_static_bytes(&self) -> TelemetryResult<Vec<u8>> {
        self.static_mapping.read_raw_bytes()
    }

    #[cfg(windows)]
    pub fn read_raw_physics(&self) -> TelemetryResult<Vec<u8>> {
        self.physics_mapping.read_raw_bytes()
    }

    #[cfg(windows)]
    pub fn read_raw_graphics(&self) -> TelemetryResult<Vec<u8>> {
        self.graphics_mapping.read_raw_bytes()
    }

    #[cfg(windows)]
    pub fn read_raw_static(&self) -> TelemetryResult<Vec<u8>> {
        self.static_mapping.read_raw_bytes()
    }

    #[cfg(windows)]
    pub fn read_control_sample(
        &mut self,
        sample_tick: u64,
        timestamp_ns: u64,
    ) -> TelemetryResult<Option<ControlSample>> {
        if !self.status()?.is_live() {
            return Ok(None);
        }

        let physics = self.physics_mapping.physics_controls()?;
        if physics.packet_id == self.last_physics_packet_id {
            return Ok(None);
        }
        self.last_physics_packet_id = physics.packet_id;

        Ok(Some(ControlSample {
            sample_tick,
            timestamp_ns,
            physics_packet_id: physics.packet_id,
            graphics_packet_id: 0,
            speed_kmh: physics.speed_kmh,
            gas: physics.gas,
            brake: physics.brake,
            clutch: physics.clutch,
            steer_angle: physics.steer_angle,
            gear: physics.gear,
            rpms: physics.rpms,
            fuel: physics.fuel,
        }))
    }

    #[cfg(not(windows))]
    pub fn read_control_sample(
        &mut self,
        _sample_tick: u64,
        _timestamp_ns: u64,
    ) -> TelemetryResult<Option<ControlSample>> {
        Ok(None)
    }

    // ---- v2 full-frame recording ----

    #[cfg(windows)]
    pub fn read_telemetry_frame(
        &mut self,
        sample_tick: u64,
        timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>> {
        if !self.status()?.is_live() {
            return Ok(None);
        }

        let phys = self.physics_mapping.physics_full()?;
        if phys.packet_id == self.last_physics_packet_id {
            return Ok(None);
        }
        self.last_physics_packet_id = phys.packet_id;

        let gfx = self.graphics_mapping.graphics_full()?;

        Ok(Some(TelemetryFrame {
            sample_tick,
            timestamp_ns,
            controls: ControlSample {
                sample_tick,
                timestamp_ns,
                physics_packet_id: phys.packet_id,
                graphics_packet_id: gfx.packet_id,
                speed_kmh: phys.speed_kmh,
                gas: phys.gas,
                brake: phys.brake,
                clutch: phys.clutch,
                steer_angle: phys.steer_angle,
                gear: phys.gear,
                rpms: phys.rpms,
                fuel: phys.fuel,
            },
            motion: MotionSample {
                sample_tick,
                timestamp_ns,
                velocity: phys.velocity,
                acc_g: phys.acc_g,
                local_velocity: phys.local_velocity,
                local_angular_vel: phys.local_angular_vel,
                heading: phys.heading,
                pitch: phys.pitch,
                roll: phys.roll,
            },
            tyres: TyreSample {
                sample_tick,
                timestamp_ns,
                wheel_slip: phys.wheel_slip,
                wheel_load: phys.wheel_load,
                wheels_pressure: phys.wheels_pressure,
                wheel_angular_speed: phys.wheel_angular_speed,
                tyre_wear: phys.tyre_wear,
                tyre_dirty_level: phys.tyre_dirty_level,
                tyre_core_temperature: phys.tyre_core_temperature,
                camber_rad: phys.camber_rad,
                suspension_travel: phys.suspension_travel,
                slip_ratio: phys.slip_ratio,
                slip_angle: phys.slip_angle,
                tyre_temp_i: phys.tyre_temp_i,
                tyre_temp_m: phys.tyre_temp_m,
                tyre_temp_o: phys.tyre_temp_o,
                tyre_temp: phys.tyre_temp,
                mz: phys.mz,
                fx: phys.fx,
                fy: phys.fy,
                suspension_damage: phys.suspension_damage,
                brake_temp: phys.brake_temp,
                brake_pressure: phys.brake_pressure,
                pad_life: phys.pad_life,
                disc_life: phys.disc_life,
                tyre_contact_point: flatten_f32_3x4(phys.tyre_contact_point),
                tyre_contact_normal: flatten_f32_3x4(phys.tyre_contact_normal),
                tyre_contact_heading: flatten_f32_3x4(phys.tyre_contact_heading),
                number_of_tyres_out: phys.number_of_tyres_out,
                front_brake_compound: phys.front_brake_compound,
                rear_brake_compound: phys.rear_brake_compound,
            },
            powertrain: PowertrainSample {
                sample_tick,
                timestamp_ns,
                turbo_boost: phys.turbo_boost,
                ballast: phys.ballast,
                kers_charge: phys.kers_charge,
                kers_input: phys.kers_input,
                kers_current_kj: phys.kers_current_kj,
                drs: phys.drs,
                tc: phys.tc,
                abs: phys.abs,
                engine_brake: phys.engine_brake,
                ers_recovery_level: phys.ers_recovery_level,
                ers_power_level: phys.ers_power_level,
                ers_heat_charging: phys.ers_heat_charging,
                ers_is_charging: phys.ers_is_charging,
                drs_available: phys.drs_available,
                drs_enabled: phys.drs_enabled,
                tc_in_action: phys.tc_in_action,
                abs_in_action: phys.abs_in_action,
                auto_shifter_on: phys.auto_shifter_on,
                current_max_rpm: phys.current_max_rpm,
                p2p_activations: phys.p2p_activations,
                p2p_status: phys.p2p_status,
                water_temp: phys.water_temp,
            },
            session: SessionSample {
                sample_tick,
                timestamp_ns,
                status: gfx.status,
                session: gfx.session,
                session_index: gfx.session_index,
                completed_laps: gfx.completed_laps,
                position: gfx.position,
                session_time_left: gfx.session_time_left,
                number_of_laps: gfx.number_of_laps,
                current_sector_index: gfx.current_sector_index,
                normalized_car_position: gfx.normalized_car_position,
                is_in_pit: gfx.is_in_pit,
                is_in_pit_lane: gfx.is_in_pit_lane,
                mandatory_pit_done: gfx.mandatory_pit_done,
                missing_mandatory_pits: gfx.missing_mandatory_pits,
                penalty_time: gfx.penalty_time,
                penalty_type: gfx.penalty,
                track_status: gfx.track_status,
                clock: gfx.clock,
                replay_time_multiplier: gfx.replay_time_multiplier,
                is_valid_lap: gfx.is_valid_lap,
                global_yellow: gfx.global_yellow,
                global_yellow1: gfx.global_yellow1,
                global_yellow2: gfx.global_yellow2,
                global_yellow3: gfx.global_yellow3,
                global_white: gfx.global_white,
                global_green: gfx.global_green,
                global_chequered: gfx.global_chequered,
                global_red: gfx.global_red,
                gap_ahead_or_tail_value: gfx.gap_ahead,
                flag: gfx.flag,
                gap_behind: gfx.gap_behind,
            },
            timing: TimingSample {
                sample_tick,
                timestamp_ns,
                i_current_time: gfx.i_current_time,
                i_last_time: gfx.i_last_time,
                i_best_time: gfx.i_best_time,
                i_split: gfx.i_split,
                last_sector_time: gfx.last_sector_time,
                i_delta_lap_time: gfx.i_delta_lap_time,
                is_delta_positive: gfx.is_delta_positive,
                i_estimated_lap_time: gfx.i_estimated_lap_time,
                fuel_estimated_laps: gfx.fuel_estimated_laps,
                fuel_x_lap: gfx.fuel_x_lap,
                used_fuel: gfx.used_fuel,
                distance_traveled: gfx.distance_traveled,
                current_time_str: gfx.current_time,
                last_time_str: gfx.last_time,
                best_time_str: gfx.best_time,
                split_str: gfx.split,
                delta_lap_time_str: gfx.delta_lap_time,
                estimated_lap_time_str: gfx.estimated_lap_time,
                observed_slot_before_i_split: 0,
            },
            car_state: CarStateSample {
                sample_tick,
                timestamp_ns,
                car_damage: phys.car_damage,
                pit_limiter_on: phys.pit_limiter_on,
                ride_height: phys.ride_height,
                ignition_on: phys.ignition_on,
                starter_engine_on: phys.starter_engine_on,
                is_engine_running: phys.is_engine_running,
                is_ai_controlled: phys.is_ai_controlled,
                cg_height: phys.cg_height,
                brake_bias: phys.brake_bias,
                rain_lights: gfx.rain_lights,
                flashing_lights: gfx.flashing_lights,
                lights_stage: gfx.lights_stage,
                wiper_lv: gfx.wiper_lv,
                driver_stint_total_time_left: gfx.driver_stint_total_time_left,
                driver_stint_time_left: gfx.driver_stint_time_left,
                rain_tyres: gfx.rain_tyres,
                current_tyre_set: gfx.current_tyre_set,
                strategy_tyre_set: gfx.strategy_tyre_set,
                track_grip_status: gfx.track_grip_status,
                tyre_compound_str: gfx.tyre_compound,
                mfd_tyre_set: gfx.mfd_tyre_set,
                mfd_fuel_to_add: gfx.mfd_fuel_to_add,
                mfd_tyre_pressure: [
                    gfx.mfd_tyre_pressure_lf,
                    gfx.mfd_tyre_pressure_rf,
                    gfx.mfd_tyre_pressure_lr,
                    gfx.mfd_tyre_pressure_rr,
                ],
                ideal_line_on: gfx.ideal_line_on,
                is_setup_menu_visible: gfx.is_setup_menu_visible,
                main_display_index: gfx.main_display_index,
                secondary_display_index: gfx.secondary_display_index,
                direction_lights_left: gfx.direction_lights_left,
                direction_lights_right: gfx.direction_lights_right,
                tc_level: gfx.tc,
                tc_cut: gfx.tc_cut,
                engine_map: gfx.engine_map,
                abs_level: gfx.abs,
                exhaust_temperature: gfx.exhaust_temperature,
                final_ff: phys.final_ff,
                performance_meter: phys.performance_meter,
                kerb_vibration: phys.kerb_vibration,
                slip_vibrations: phys.slip_vibrations,
                g_vibrations: phys.g_vibrations,
                abs_vibrations: phys.abs_vibrations,
            },
            environment: EnvironmentSample {
                sample_tick,
                timestamp_ns,
                air_density: phys.air_density,
                air_temp: phys.air_temp,
                road_temp: phys.road_temp,
                wind_speed: gfx.wind_speed,
                wind_direction: gfx.wind_direction,
                surface_grip: gfx.surface_grip,
                rain_intensity: gfx.rain_intensity,
                rain_intensity_in_10min: gfx.rain_intensity_in_10min,
                rain_intensity_in_30min: gfx.rain_intensity_in_30min,
            },
            other_cars: OtherCarsSample {
                sample_tick,
                timestamp_ns,
                active_cars: gfx.active_cars,
                player_car_id: gfx.player_car_id,
                car_coordinates: gfx.car_coordinates.iter().flat_map(|c| c.to_vec()).collect(),
                car_id: gfx.car_id.to_vec(),
            },
        }))
    }

    #[cfg(not(windows))]
    pub fn read_telemetry_frame(
        &mut self,
        _sample_tick: u64,
        _timestamp_ns: u64,
    ) -> TelemetryResult<Option<TelemetryFrame>> {
        Err(TelemetryError::InvalidArgument(
            "ACC shared memory is only available on Windows".to_string(),
        ))
    }
}

#[cfg(windows)]
fn flatten_f32_3x4(arr: [[f32; 3]; 4]) -> [f32; 12] {
    let mut out = [0.0f32; 12];
    for i in 0..4 {
        out[i * 3] = arr[i][0];
        out[i * 3 + 1] = arr[i][1];
        out[i * 3 + 2] = arr[i][2];
    }
    out
}

// ---------------------------------------------------------------------------
// Windows shared memory structs
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct SPageFilePhysicsControls {
    packet_id: i32,
    gas: f32,
    brake: f32,
    fuel: f32,
    gear: i32,
    rpms: i32,
    steer_angle: f32,
    speed_kmh: f32,
    velocity: [f32; 3],
    acc_g: [f32; 3],
    wheel_slip: [f32; 4],
    wheel_load: [f32; 4],
    wheels_pressure: [f32; 4],
    wheel_angular_speed: [f32; 4],
    tyre_wear: [f32; 4],
    tyre_dirty_level: [f32; 4],
    tyre_core_temperature: [f32; 4],
    camber_rad: [f32; 4],
    suspension_travel: [f32; 4],
    drs: f32,
    tc: f32,
    heading: f32,
    pitch: f32,
    roll: f32,
    cg_height: f32,
    car_damage: [f32; 5],
    number_of_tyres_out: i32,
    pit_limiter_on: i32,
    abs: f32,
    kers_charge: f32,
    kers_input: f32,
    auto_shifter_on: i32,
    ride_height: [f32; 2],
    turbo_boost: f32,
    ballast: f32,
    air_density: f32,
    air_temp: f32,
    road_temp: f32,
    local_angular_vel: [f32; 3],
    final_ff: f32,
    performance_meter: f32,
    engine_brake: i32,
    ers_recovery_level: i32,
    ers_power_level: i32,
    ers_heat_charging: i32,
    ers_is_charging: i32,
    kers_current_kj: f32,
    drs_available: i32,
    drs_enabled: i32,
    brake_temp: [f32; 4],
    clutch: f32,
    tyre_temp_i: [f32; 4],
    tyre_temp_m: [f32; 4],
    tyre_temp_o: [f32; 4],
    is_ai_controlled: i32,
    tyre_contact_point: [[f32; 3]; 4],
    tyre_contact_normal: [[f32; 3]; 4],
    tyre_contact_heading: [[f32; 3]; 4],
    brake_bias: f32,
    local_velocity: [f32; 3],
    p2p_activations: i32,
    p2p_status: i32,
    current_max_rpm: i32,
    mz: [f32; 4],
    fx: [f32; 4],
    fy: [f32; 4],
    slip_ratio: [f32; 4],
    slip_angle: [f32; 4],
    tc_in_action: i32,
    abs_in_action: i32,
    suspension_damage: [f32; 4],
    tyre_temp: [f32; 4],
    water_temp: f32,
    brake_pressure: [f32; 4],
    front_brake_compound: i32,
    rear_brake_compound: i32,
    pad_life: [f32; 4],
    disc_life: [f32; 4],
    ignition_on: i32,
    starter_engine_on: i32,
    is_engine_running: i32,
    kerb_vibration: f32,
    slip_vibrations: f32,
    g_vibrations: f32,
    abs_vibrations: f32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct SPageFileGraphicsRaw {
    packet_id: i32,
    status: i32,
    session: i32,
    current_time: [u16; 15],
    last_time: [u16; 15],
    best_time: [u16; 15],
    split: [u16; 15],
    completed_laps: i32,
    position: i32,
    i_current_time: i32,
    i_last_time: i32,
    i_best_time: i32,
    session_time_left: f32,
    distance_traveled: f32,
    is_in_pit: i32,
    current_sector_index: i32,
    last_sector_time: i32,
    number_of_laps: i32,
    tyre_compound: [u16; 33],
    replay_time_multiplier: f32,
    normalized_car_position: f32,
    active_cars: i32,
    car_coordinates: [[f32; 3]; 60],
    car_id: [i32; 60],
    player_car_id: i32,
    penalty_time: f32,
    flag: i32,          // AC_FLAG_TYPE
    penalty: i32,       // AC_PENALTY_TYPE (was missing - caused 4-byte shift)
    ideal_line_on: i32,
    is_in_pit_lane: i32,
    surface_grip: f32,
    mandatory_pit_done: i32,
    wind_speed: f32,
    wind_direction: f32,
    is_setup_menu_visible: i32,
    main_display_index: i32,
    secondary_display_index: i32,
    tc: i32,
    tc_cut: i32,
    engine_map: i32,
    abs: i32,
    fuel_x_lap: f32,
    rain_lights: i32,
    flashing_lights: i32,
    lights_stage: i32,
    exhaust_temperature: f32,
    wiper_lv: i32,
    driver_stint_total_time_left: i32,
    driver_stint_time_left: i32,
    rain_tyres: i32,
    session_index: i32,
    used_fuel: f32,
    delta_lap_time: [u16; 15],
    i_delta_lap_time: i32,
    estimated_lap_time: [u16; 15],
    i_estimated_lap_time: i32,
    is_delta_positive: i32,
    i_split: i32,
    is_valid_lap: i32,
    fuel_estimated_laps: f32,
    track_status: [u16; 33],
    missing_mandatory_pits: i32,
    clock: f32,
    direction_lights_left: i32,
    direction_lights_right: i32,
    global_yellow: i32,
    global_yellow1: i32,
    global_yellow2: i32,
    global_yellow3: i32,
    global_white: i32,
    global_green: i32,
    global_chequered: i32,
    global_red: i32,
    mfd_tyre_set: i32,
    mfd_fuel_to_add: f32,
    mfd_tyre_pressure_lf: f32,
    mfd_tyre_pressure_rf: f32,
    mfd_tyre_pressure_lr: f32,
    mfd_tyre_pressure_rr: f32,
    track_grip_status: i32,
    rain_intensity: i32,
    rain_intensity_in_10min: i32,
    rain_intensity_in_30min: i32,
    current_tyre_set: i32,
    strategy_tyre_set: i32,
    gap_ahead: i32,
    gap_behind: i32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SPageFileStatic {
    pub sm_version: [u16; 15],
    pub ac_version: [u16; 15],
    pub number_of_sessions: i32,
    pub num_cars: i32,
    pub car_model: [u16; 33],
    pub track: [u16; 33],
    pub player_name: [u16; 33],
    pub player_surname: [u16; 33],
    pub player_nick: [u16; 33],
    pub sector_count: i32,
    pub max_torque: f32,
    pub max_power: f32,
    pub max_rpm: i32,
    pub max_fuel: f32,
    pub suspension_max_travel: [f32; 4],
    pub tyre_radius: [f32; 4],
    pub max_turbo_boost: f32,
    pub deprecated_1: f32,
    pub deprecated_2: f32,
    pub penalties_enabled: i32,
    pub aid_fuel_rate: f32,
    pub aid_tire_rate: f32,
    pub aid_mechanical_damage: f32,
    pub aid_allow_tyre_blankets: i32,
    pub aid_stability: f32,
    pub aid_auto_clutch: i32,
    pub aid_auto_blip: i32,
    pub has_drs: i32,
    pub has_ers: i32,
    pub has_kers: i32,
    pub kers_max_j: f32,
    pub engine_brake_settings_count: i32,
    pub ers_power_controller_count: i32,
    pub track_spline_length: f32,
    pub track_configuration: [u16; 33],
    pub ers_max_j: f32,
    pub is_timed_race: i32,
    pub has_extra_lap: i32,
    pub car_skin: [u16; 33],
    pub reversed_grid_positions: i32,
    pub pit_window_start: i32,
    pub pit_window_end: i32,
    pub is_online: i32,
    pub dry_tyres_name: [u16; 33],
    pub wet_tyres_name: [u16; 33],
}

impl SPageFileStatic {
    /// Parse from raw bytes read from ACC shared-memory static page.
    /// The byte slice may be shorter than the full struct (older game versions);
    /// missing fields default to zero.
    pub fn from_raw(bytes: &[u8]) -> Self {
        let mut out = Self::default();
        let field_size = std::mem::size_of::<Self>();
        let copy_len = bytes.len().min(field_size);
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                &mut out as *mut Self as *mut u8,
                copy_len,
            );
        }
        out
    }

    pub fn car_model_str(&self) -> String {
        utf16_to_string(&self.car_model)
    }

    pub fn track_str(&self) -> String {
        utf16_to_string(&self.track)
    }

    pub fn sm_version_str(&self) -> String {
        utf16_to_string(&self.sm_version)
    }

    pub fn ac_version_str(&self) -> String {
        utf16_to_string(&self.ac_version)
    }
}

impl Default for SPageFileStatic {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

fn utf16_to_string(arr: &[u16]) -> String {
    String::from_utf16_lossy(
        &arr.iter()
            .take_while(|&&c| c != 0)
            .copied()
            .collect::<Vec<u16>>(),
    )
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct PhysicsControlsSnapshot {
    packet_id: i32,
    gas: f32,
    brake: f32,
    fuel: f32,
    gear: i32,
    rpms: i32,
    steer_angle: f32,
    speed_kmh: f32,
    clutch: f32,
}

#[cfg(windows)]
struct WindowsSharedMemory {
    handle: *mut std::ffi::c_void,
    view: *const std::ffi::c_void,
    mapped_size: usize,
}

#[cfg(windows)]
unsafe impl Send for WindowsSharedMemory {}

#[cfg(windows)]
impl WindowsSharedMemory {
    fn open_physics() -> TelemetryResult<Self> {
        Self::open(
            "Local\\acpmf_physics",
            std::mem::size_of::<SPageFilePhysicsControls>(),
        )
    }

    fn open_graphics() -> TelemetryResult<Self> {
        Self::open(
            "Local\\acpmf_graphics",
            std::mem::size_of::<SPageFileGraphicsRaw>(),
        )
    }

    fn open_static() -> TelemetryResult<Self> {
        Self::open(
            "Local\\acpmf_static",
            std::mem::size_of::<SPageFileStatic>(),
        )
    }

    fn open(mapping_name: &str, mapped_size: usize) -> TelemetryResult<Self> {
        let mapping_name_wide = to_wide(mapping_name);
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, mapping_name_wide.as_ptr()) };
        if handle.is_null() {
            return Err(TelemetryError::Io(std::io::Error::last_os_error()));
        }

        let view = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, mapped_size) };
        if view.is_null() {
            unsafe {
                CloseHandle(handle);
            }
            return Err(TelemetryError::Io(std::io::Error::last_os_error()));
        }

        Ok(Self {
            handle,
            view,
            mapped_size,
        })
    }

    fn status(&self) -> TelemetryResult<i32> {
        if self.mapped_size < std::mem::size_of::<SPageFileGraphicsRaw>() {
            return Err(TelemetryError::InvalidFormat(
                "ACC graphics mapping size too small".to_string(),
            ));
        }
        let graphics = unsafe { *(self.view as *const SPageFileGraphicsRaw) };
        Ok(graphics.status)
    }

    fn read_raw_bytes(&self) -> TelemetryResult<Vec<u8>> {
        if self.view.is_null() { return Err(TelemetryError::Io(std::io::Error::new(std::io::ErrorKind::Other, "null view"))); }
        let slice = unsafe { std::slice::from_raw_parts(self.view as *const u8, self.mapped_size) };
        Ok(slice.to_vec())
    }

    fn physics_controls(&self) -> TelemetryResult<PhysicsControlsSnapshot> {
        if self.mapped_size < std::mem::size_of::<SPageFilePhysicsControls>() {
            return Err(TelemetryError::InvalidFormat(
                "ACC physics mapping size too small".to_string(),
            ));
        }
        let physics = unsafe { *(self.view as *const SPageFilePhysicsControls) };
        Ok(PhysicsControlsSnapshot {
            packet_id: physics.packet_id,
            gas: physics.gas,
            brake: physics.brake,
            fuel: physics.fuel,
            gear: physics.gear,
            rpms: physics.rpms,
            steer_angle: physics.steer_angle,
            speed_kmh: physics.speed_kmh,
            clutch: physics.clutch,
        })
    }

    fn physics_full(&self) -> TelemetryResult<SPageFilePhysicsControls> {
        if self.mapped_size < std::mem::size_of::<SPageFilePhysicsControls>() {
            return Err(TelemetryError::InvalidFormat(
                "ACC physics mapping size too small".to_string(),
            ));
        }
        Ok(unsafe { *(self.view as *const SPageFilePhysicsControls) })
    }

    fn graphics_full(&self) -> TelemetryResult<SPageFileGraphicsRaw> {
        if self.mapped_size < std::mem::size_of::<SPageFileGraphicsRaw>() {
            return Err(TelemetryError::InvalidFormat(
                "ACC graphics mapping size too small".to_string(),
            ));
        }
        Ok(unsafe { *(self.view as *const SPageFileGraphicsRaw) })
    }

    fn static_snapshot(&self) -> TelemetryResult<AccSessionInfo> {
        if self.mapped_size < std::mem::size_of::<SPageFileStatic>() {
            return Err(TelemetryError::InvalidFormat(
                "ACC static mapping size too small".to_string(),
            ));
        }
        let sp = unsafe { *(self.view as *const SPageFileStatic) };
        let track_name = clean_name(&wide_z_to_string(&sp.track), "unknown_track");
        let car_model = clean_name(&wide_z_to_string(&sp.car_model), "unknown_car");
        Ok(AccSessionInfo {
            track_name,
            car_model,
        })
    }

    fn static_full(&self) -> TelemetryResult<SPageFileStatic> {
        if self.mapped_size < std::mem::size_of::<SPageFileStatic>() {
            return Err(TelemetryError::InvalidFormat(
                "ACC static mapping size too small".to_string(),
            ));
        }
        Ok(unsafe { *(self.view as *const SPageFileStatic) })
    }
}

#[cfg(windows)]
impl Drop for WindowsSharedMemory {
    fn drop(&mut self) {
        if !self.view.is_null() {
            unsafe {
                UnmapViewOfFile(self.view);
            }
            self.view = std::ptr::null();
        }
        if !self.handle.is_null() {
            unsafe {
                CloseHandle(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

#[cfg(windows)]
fn clean_name(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("unknown") {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(windows)]
fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn wide_z_to_string(input: &[u16]) -> String {
    let end = input.iter().position(|ch| *ch == 0).unwrap_or(input.len());
    String::from_utf16_lossy(&input[..end]).trim().to_string()
}

#[cfg(windows)]
const FILE_MAP_READ: u32 = 0x0004;

#[cfg(windows)]
pub const RAW_PHYSICS_SIZE: usize = std::mem::size_of::<SPageFilePhysicsControls>();
#[cfg(windows)]
pub const RAW_GRAPHICS_SIZE: usize = std::mem::size_of::<SPageFileGraphicsRaw>();
#[cfg(windows)]
pub const RAW_STATIC_SIZE: usize = std::mem::size_of::<SPageFileStatic>();

#[cfg(windows)]
pub fn parse_raw_frame(
    sample_tick: u64,
    timestamp_ns: u64,
    physics_bytes: &[u8],
    graphics_bytes: &[u8],
) -> TelemetryResult<TelemetryFrame> {
    let phys_size = RAW_PHYSICS_SIZE;
    let graph_size = RAW_GRAPHICS_SIZE;
    if physics_bytes.len() < phys_size {
        return Err(TelemetryError::InvalidFormat("physics page too small".into()));
    }
    if graphics_bytes.len() < graph_size {
        return Err(TelemetryError::InvalidFormat("graphics page too small".into()));
    }
    let phys: &SPageFilePhysicsControls = unsafe { &*(physics_bytes.as_ptr() as *const SPageFilePhysicsControls) };
    let gfx: &SPageFileGraphicsRaw = unsafe { &*(graphics_bytes.as_ptr() as *const SPageFileGraphicsRaw) };

    Ok(TelemetryFrame {
        sample_tick,
        timestamp_ns,
        controls: ControlSample {
            sample_tick, timestamp_ns,
            physics_packet_id: phys.packet_id, graphics_packet_id: gfx.packet_id,
            speed_kmh: phys.speed_kmh, gas: phys.gas, brake: phys.brake,
            clutch: phys.clutch, steer_angle: phys.steer_angle,
            gear: phys.gear, rpms: phys.rpms, fuel: phys.fuel,
        },
        motion: MotionSample {
            sample_tick, timestamp_ns,
            velocity: phys.velocity, acc_g: phys.acc_g,
            local_velocity: phys.local_velocity, local_angular_vel: phys.local_angular_vel,
            heading: phys.heading, pitch: phys.pitch, roll: phys.roll,
        },
        tyres: TyreSample {
            sample_tick, timestamp_ns,
            wheel_slip: phys.wheel_slip, wheel_load: phys.wheel_load,
            wheels_pressure: phys.wheels_pressure, wheel_angular_speed: phys.wheel_angular_speed,
            tyre_wear: phys.tyre_wear, tyre_dirty_level: phys.tyre_dirty_level,
            tyre_core_temperature: phys.tyre_core_temperature, camber_rad: phys.camber_rad,
            suspension_travel: phys.suspension_travel, slip_ratio: phys.slip_ratio,
            slip_angle: phys.slip_angle, tyre_temp_i: phys.tyre_temp_i,
            tyre_temp_m: phys.tyre_temp_m, tyre_temp_o: phys.tyre_temp_o,
            tyre_temp: phys.tyre_temp, mz: phys.mz, fx: phys.fx, fy: phys.fy,
            suspension_damage: phys.suspension_damage, brake_temp: phys.brake_temp,
            brake_pressure: phys.brake_pressure, pad_life: phys.pad_life, disc_life: phys.disc_life,
            tyre_contact_point: unsafe { std::mem::transmute(phys.tyre_contact_point) },
            tyre_contact_normal: unsafe { std::mem::transmute(phys.tyre_contact_normal) },
            tyre_contact_heading: unsafe { std::mem::transmute(phys.tyre_contact_heading) },
            number_of_tyres_out: phys.number_of_tyres_out,
            front_brake_compound: phys.front_brake_compound,
            rear_brake_compound: phys.rear_brake_compound,
        },
        powertrain: PowertrainSample {
            sample_tick, timestamp_ns,
            turbo_boost: phys.turbo_boost, ballast: phys.ballast, kers_charge: phys.kers_charge,
            kers_input: phys.kers_input, kers_current_kj: phys.kers_current_kj,
            drs: phys.drs, tc: phys.tc, abs: phys.abs,
            engine_brake: phys.engine_brake, ers_recovery_level: phys.ers_recovery_level,
            ers_power_level: phys.ers_power_level, ers_heat_charging: phys.ers_heat_charging,
            ers_is_charging: phys.ers_is_charging, drs_available: phys.drs_available,
            drs_enabled: phys.drs_enabled, tc_in_action: phys.tc_in_action,
            abs_in_action: phys.abs_in_action, auto_shifter_on: phys.auto_shifter_on,
            current_max_rpm: phys.current_max_rpm, p2p_activations: phys.p2p_activations,
            p2p_status: phys.p2p_status, water_temp: phys.water_temp,
        },
        session: SessionSample {
            sample_tick, timestamp_ns,
            status: gfx.status, session: gfx.session, session_index: gfx.session_index,
            completed_laps: gfx.completed_laps, position: gfx.position,
            session_time_left: gfx.session_time_left, number_of_laps: gfx.number_of_laps,
            current_sector_index: gfx.current_sector_index,
            normalized_car_position: gfx.normalized_car_position,
            is_in_pit: gfx.is_in_pit, is_in_pit_lane: gfx.is_in_pit_lane,
            mandatory_pit_done: gfx.mandatory_pit_done,
            missing_mandatory_pits: gfx.missing_mandatory_pits,
            penalty_time: gfx.penalty_time, penalty_type: gfx.penalty,
            track_status: gfx.track_status, clock: gfx.clock,
            replay_time_multiplier: gfx.replay_time_multiplier,
            is_valid_lap: gfx.is_valid_lap,
            global_yellow: gfx.global_yellow, global_yellow1: gfx.global_yellow1,
            global_yellow2: gfx.global_yellow2, global_yellow3: gfx.global_yellow3,
            global_white: gfx.global_white, global_green: gfx.global_green,
            global_chequered: gfx.global_chequered, global_red: gfx.global_red,
            gap_ahead_or_tail_value: gfx.gap_ahead,
            flag: gfx.flag,
            gap_behind: gfx.gap_behind,
        },
        timing: TimingSample {
            sample_tick, timestamp_ns,
            i_current_time: gfx.i_current_time, i_last_time: gfx.i_last_time,
            i_best_time: gfx.i_best_time, i_split: gfx.i_split,
            last_sector_time: gfx.last_sector_time,
            i_delta_lap_time: gfx.i_delta_lap_time,
            is_delta_positive: gfx.is_delta_positive,
            i_estimated_lap_time: gfx.i_estimated_lap_time,
            fuel_estimated_laps: gfx.fuel_estimated_laps,
            fuel_x_lap: gfx.fuel_x_lap, used_fuel: gfx.used_fuel,
            distance_traveled: gfx.distance_traveled,
            current_time_str: gfx.current_time, last_time_str: gfx.last_time,
            best_time_str: gfx.best_time, split_str: gfx.split,
            delta_lap_time_str: gfx.delta_lap_time,
            estimated_lap_time_str: gfx.estimated_lap_time,
            observed_slot_before_i_split: 0,
        },
        car_state: CarStateSample {
            sample_tick, timestamp_ns,
            car_damage: phys.car_damage, pit_limiter_on: phys.pit_limiter_on,
            ride_height: phys.ride_height, ignition_on: phys.ignition_on,
            starter_engine_on: phys.starter_engine_on, is_engine_running: phys.is_engine_running,
            is_ai_controlled: phys.is_ai_controlled, cg_height: phys.cg_height,
            brake_bias: phys.brake_bias, rain_lights: gfx.rain_lights,
            flashing_lights: gfx.flashing_lights, lights_stage: gfx.lights_stage,
            wiper_lv: gfx.wiper_lv,
            driver_stint_total_time_left: gfx.driver_stint_total_time_left,
            driver_stint_time_left: gfx.driver_stint_time_left,
            rain_tyres: gfx.rain_tyres, current_tyre_set: gfx.current_tyre_set,
            strategy_tyre_set: gfx.strategy_tyre_set, track_grip_status: gfx.track_grip_status,
            tyre_compound_str: gfx.tyre_compound,
            mfd_tyre_set: gfx.mfd_tyre_set, mfd_fuel_to_add: gfx.mfd_fuel_to_add,
            mfd_tyre_pressure: [gfx.mfd_tyre_pressure_lf, gfx.mfd_tyre_pressure_rf,
                gfx.mfd_tyre_pressure_lr, gfx.mfd_tyre_pressure_rr],
            ideal_line_on: gfx.ideal_line_on,
            is_setup_menu_visible: gfx.is_setup_menu_visible,
            main_display_index: gfx.main_display_index,
            secondary_display_index: gfx.secondary_display_index,
            direction_lights_left: gfx.direction_lights_left,
            direction_lights_right: gfx.direction_lights_right,
            tc_level: gfx.tc, tc_cut: gfx.tc_cut,
            engine_map: gfx.engine_map, abs_level: gfx.abs,
            exhaust_temperature: gfx.exhaust_temperature,
            final_ff: phys.final_ff,
            performance_meter: phys.performance_meter,
            kerb_vibration: phys.kerb_vibration, slip_vibrations: phys.slip_vibrations,
            g_vibrations: phys.g_vibrations, abs_vibrations: phys.abs_vibrations,
        },
        environment: EnvironmentSample {
            sample_tick, timestamp_ns,
            air_density: phys.air_density, air_temp: phys.air_temp, road_temp: phys.road_temp,
            wind_speed: gfx.wind_speed, wind_direction: gfx.wind_direction,
            surface_grip: gfx.surface_grip,
            rain_intensity: gfx.rain_intensity,
            rain_intensity_in_10min: gfx.rain_intensity_in_10min,
            rain_intensity_in_30min: gfx.rain_intensity_in_30min,
        },
        other_cars: OtherCarsSample {
            sample_tick, timestamp_ns,
            active_cars: gfx.active_cars, player_car_id: gfx.player_car_id,
            car_coordinates: gfx.car_coordinates.iter().flat_map(|c| c.to_vec()).collect(),
            car_id: gfx.car_id.to_vec(),
        },
    })
}

#[cfg(windows)]
extern "system" {
    fn OpenFileMappingW(
        dwDesiredAccess: u32,
        bInheritHandle: i32,
        lpName: *const u16,
    ) -> *mut std::ffi::c_void;
    fn MapViewOfFile(
        hFileMappingObject: *mut std::ffi::c_void,
        dwDesiredAccess: u32,
        dwFileOffsetHigh: u32,
        dwFileOffsetLow: u32,
        dwNumberOfBytesToMap: usize,
    ) -> *const std::ffi::c_void;
    fn UnmapViewOfFile(lpBaseAddress: *const std::ffi::c_void) -> i32;
    fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
}