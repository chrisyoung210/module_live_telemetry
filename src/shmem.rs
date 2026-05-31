use crate::error::{TelemetryError, TelemetryResult};
use crate::types::ControlSample;

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
}

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
    penalty_type: i32,
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
struct SPageFileStaticIdentity {
    sm_version: [u16; 15],
    ac_version: [u16; 15],
    number_of_sessions: i32,
    num_cars: i32,
    car_model: [u16; 33],
    track: [u16; 33],
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
            std::mem::size_of::<SPageFileStaticIdentity>(),
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

    fn static_snapshot(&self) -> TelemetryResult<AccSessionInfo> {
        if self.mapped_size < std::mem::size_of::<SPageFileStaticIdentity>() {
            return Err(TelemetryError::InvalidFormat(
                "ACC static mapping size too small".to_string(),
            ));
        }
        let static_page = unsafe { *(self.view as *const SPageFileStaticIdentity) };
        let track_name = clean_name(&wide_z_to_string(&static_page.track), "unknown_track");
        let car_model = clean_name(&wide_z_to_string(&static_page.car_model), "unknown_car");
        Ok(AccSessionInfo {
            track_name,
            car_model,
        })
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
#[link(name = "kernel32")]
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
