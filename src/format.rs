use crate::error::{TelemetryError, TelemetryResult};
use std::io::{Read, Write};

// ---------------------------------------------------------------------------
// File-level constants (unchanged binary layout)
// ---------------------------------------------------------------------------
pub const MAGIC: [u8; 8] = *b"ACTL\r\n\x1A\n";
pub const FORMAT_VERSION: u16 = 2;
pub const HEADER_SIZE: u16 = 128;
pub const TIMEBASE_HZ: u32 = 1_000_000_000;
pub const SCHEMA_HASH: u64 = 0x4143_544c_0000_0002;

pub const CHUNK_MAGIC: [u8; 4] = *b"CHNK";
pub const CHUNK_HEADER_SIZE: u16 = 72;
pub const COLUMN_ENTRY_SIZE: usize = 40;
pub const INDEX_MAGIC: [u8; 4] = *b"INDX";
pub const FOOTER_MAGIC: [u8; 4] = *b"FOOT";
pub const META_MAGIC: [u8; 4] = *b"META";
pub const SCHEMA_MAGIC: [u8; 4] = *b"SCHM";

pub const TYPE_U64: u8 = 1;
pub const TYPE_F32: u8 = 2;
pub const TYPE_I32: u8 = 3;
pub const TYPE_BYTES: u8 = 4;
pub const CODEC_PLAIN_LE: u8 = 0;

// ---------------------------------------------------------------------------
// Cluster IDs
// ---------------------------------------------------------------------------
pub const CLUSTER_CONTROLS: u16 = 0x0100;
pub const CLUSTER_MOTION: u16 = 0x0200;
pub const CLUSTER_TYRES: u16 = 0x0300;
pub const CLUSTER_POWERTRAIN: u16 = 0x0400;
pub const CLUSTER_SESSION: u16 = 0x0500;
pub const CLUSTER_TIMING: u16 = 0x0600;
pub const CLUSTER_CAR_STATE: u16 = 0x0700;
pub const CLUSTER_ENVIRONMENT: u16 = 0x0800;
pub const CLUSTER_OTHER_CARS: u16 = 0x0900;



// ---------------------------------------------------------------------------
// Column IDs
// ---------------------------------------------------------------------------
// Common (shared across all clusters)
pub const COL_SAMPLE_TICK: u16 = 1;
pub const COL_TIMESTAMP_NS: u16 = 2;

// Controls (0x0100)
pub const COL_PHYSICS_PACKET_ID: u16 = 3;
pub const COL_GRAPHICS_PACKET_ID: u16 = 4;
pub const COL_SPEED_KMH: u16 = 10;
pub const COL_GAS: u16 = 11;
pub const COL_BRAKE: u16 = 12;
pub const COL_CLUTCH: u16 = 13;
pub const COL_STEER_ANGLE: u16 = 14;
pub const COL_GEAR: u16 = 15;
pub const COL_RPMS: u16 = 16;
pub const COL_FUEL: u16 = 17;

// Motion (0x0200)
pub const COL_VELOCITY: u16 = 20;
pub const COL_ACC_G: u16 = 21;
pub const COL_LOCAL_VELOCITY: u16 = 22;
pub const COL_LOCAL_ANGULAR_VEL: u16 = 23;
pub const COL_HEADING: u16 = 24;
pub const COL_PITCH: u16 = 25;
pub const COL_ROLL: u16 = 26;

// Tyres (0x0300)
pub const COL_WHEEL_SLIP: u16 = 30;
pub const COL_WHEEL_LOAD: u16 = 31;
pub const COL_WHEELS_PRESSURE: u16 = 32;
pub const COL_WHEEL_ANGULAR_SPEED: u16 = 33;
pub const COL_TYRE_WEAR: u16 = 34;
pub const COL_TYRE_DIRTY_LEVEL: u16 = 35;
pub const COL_TYRE_CORE_TEMPERATURE: u16 = 36;
pub const COL_CAMBER_RAD: u16 = 37;
pub const COL_SUSPENSION_TRAVEL: u16 = 38;
pub const COL_SLIP_RATIO: u16 = 39;
pub const COL_SLIP_ANGLE: u16 = 40;
pub const COL_TYRE_TEMP_I: u16 = 41;
pub const COL_TYRE_TEMP_M: u16 = 42;
pub const COL_TYRE_TEMP_O: u16 = 43;
pub const COL_TYRE_TEMP: u16 = 44;
pub const COL_MZ: u16 = 45;
pub const COL_FX: u16 = 46;
pub const COL_FY: u16 = 47;
pub const COL_SUSPENSION_DAMAGE: u16 = 48;
pub const COL_BRAKE_TEMP: u16 = 49;
pub const COL_BRAKE_PRESSURE: u16 = 50;
pub const COL_PAD_LIFE: u16 = 51;
pub const COL_DISC_LIFE: u16 = 52;
pub const COL_TYRE_CONTACT_POINT: u16 = 53;
pub const COL_TYRE_CONTACT_NORMAL: u16 = 54;
pub const COL_TYRE_CONTACT_HEADING: u16 = 55;
pub const COL_NUMBER_OF_TYRES_OUT: u16 = 56;
pub const COL_FRONT_BRAKE_COMPOUND: u16 = 57;
pub const COL_REAR_BRAKE_COMPOUND: u16 = 58;

// Powertrain (0x0400)
pub const COL_TURBO_BOOST: u16 = 60;
pub const COL_BALLAST: u16 = 61;
pub const COL_KERS_CHARGE: u16 = 62;
pub const COL_KERS_INPUT: u16 = 63;
pub const COL_KERS_CURRENT_KJ: u16 = 64;
pub const COL_DRS: u16 = 65;
pub const COL_TC_PHYSICS: u16 = 66;
pub const COL_ABS_PHYSICS: u16 = 67;
pub const COL_ENGINE_BRAKE: u16 = 68;
pub const COL_ERS_RECOVERY_LEVEL: u16 = 69;
pub const COL_ERS_POWER_LEVEL: u16 = 70;
pub const COL_ERS_HEAT_CHARGING: u16 = 71;
pub const COL_ERS_IS_CHARGING: u16 = 72;
pub const COL_DRS_AVAILABLE: u16 = 73;
pub const COL_DRS_ENABLED: u16 = 74;
pub const COL_TC_IN_ACTION: u16 = 75;
pub const COL_ABS_IN_ACTION: u16 = 76;
pub const COL_AUTO_SHIFTER_ON: u16 = 77;
pub const COL_CURRENT_MAX_RPM: u16 = 78;
pub const COL_P2P_ACTIVATIONS: u16 = 79;
pub const COL_P2P_STATUS: u16 = 80;
pub const COL_WATER_TEMP: u16 = 81;

// Session (0x0500)
pub const COL_STATUS: u16 = 90;
pub const COL_SESSION: u16 = 91;
pub const COL_SESSION_INDEX: u16 = 92;
pub const COL_COMPLETED_LAPS: u16 = 93;
pub const COL_POSITION: u16 = 94;
pub const COL_SESSION_TIME_LEFT: u16 = 95;
pub const COL_NUMBER_OF_LAPS: u16 = 96;
pub const COL_CURRENT_SECTOR_INDEX: u16 = 97;
pub const COL_NORMALIZED_CAR_POSITION: u16 = 98;
pub const COL_IS_IN_PIT: u16 = 99;
pub const COL_IS_IN_PIT_LANE: u16 = 100;
pub const COL_MANDATORY_PIT_DONE: u16 = 101;
pub const COL_MISSING_MANDATORY_PITS: u16 = 102;
pub const COL_PENALTY_TIME: u16 = 103;
pub const COL_PENALTY_TYPE: u16 = 104;
pub const COL_TRACK_STATUS: u16 = 105;
pub const COL_CLOCK: u16 = 106;
pub const COL_REPLAY_TIME_MULTIPLIER: u16 = 107;
pub const COL_IS_VALID_LAP: u16 = 108;
pub const COL_GLOBAL_YELLOW: u16 = 109;
pub const COL_GLOBAL_YELLOW1: u16 = 110;
pub const COL_GLOBAL_YELLOW2: u16 = 111;
pub const COL_GLOBAL_YELLOW3: u16 = 112;
pub const COL_GLOBAL_WHITE: u16 = 113;
pub const COL_GLOBAL_GREEN: u16 = 114;
pub const COL_GLOBAL_CHEQUERED: u16 = 115;
pub const COL_GLOBAL_RED: u16 = 116;
pub const COL_GAP_AHEAD_OR_TAIL: u16 = 117;

// Timing (0x0600)
pub const COL_I_CURRENT_TIME: u16 = 120;
pub const COL_I_LAST_TIME: u16 = 121;
pub const COL_I_BEST_TIME: u16 = 122;
pub const COL_I_SPLIT: u16 = 123;
pub const COL_LAST_SECTOR_TIME: u16 = 124;
pub const COL_I_DELTA_LAP_TIME: u16 = 125;
pub const COL_IS_DELTA_POSITIVE: u16 = 126;
pub const COL_I_ESTIMATED_LAP_TIME: u16 = 127;
pub const COL_FUEL_ESTIMATED_LAPS: u16 = 128;
pub const COL_FUEL_X_LAP: u16 = 129;
pub const COL_USED_FUEL: u16 = 130;
pub const COL_DISTANCE_TRAVELED: u16 = 131;
pub const COL_CURRENT_TIME_STR: u16 = 132;
pub const COL_LAST_TIME_STR: u16 = 133;
pub const COL_BEST_TIME_STR: u16 = 134;
pub const COL_SPLIT_STR: u16 = 135;
pub const COL_DELTA_LAP_TIME_STR: u16 = 136;
pub const COL_ESTIMATED_LAP_TIME_STR: u16 = 137;
pub const COL_OBSERVED_SLOT_BEFORE_I_SPLIT: u16 = 138;

// CarState (0x0700)
pub const COL_CAR_DAMAGE: u16 = 150;
pub const COL_PIT_LIMITER_ON: u16 = 151;
pub const COL_RIDE_HEIGHT: u16 = 152;
pub const COL_IGNITION_ON: u16 = 153;
pub const COL_STARTER_ENGINE_ON: u16 = 154;
pub const COL_IS_ENGINE_RUNNING: u16 = 155;
pub const COL_IS_AI_CONTROLLED: u16 = 156;
pub const COL_CG_HEIGHT: u16 = 157;
pub const COL_BRAKE_BIAS: u16 = 158;
pub const COL_RAIN_LIGHTS: u16 = 159;
pub const COL_FLASHING_LIGHTS: u16 = 160;
pub const COL_LIGHTS_STAGE: u16 = 161;
pub const COL_WIPER_LV: u16 = 162;
pub const COL_DRIVER_STINT_TOTAL_TIME_LEFT: u16 = 163;
pub const COL_DRIVER_STINT_TIME_LEFT: u16 = 164;
pub const COL_RAIN_TYRES: u16 = 165;
pub const COL_CURRENT_TYRE_SET: u16 = 166;
pub const COL_STRATEGY_TYRE_SET: u16 = 167;
pub const COL_TRACK_GRIP_STATUS: u16 = 168;
pub const COL_TYRE_COMPOUND_STR: u16 = 169;
pub const COL_MFD_TYRE_SET: u16 = 170;
pub const COL_MFD_FUEL_TO_ADD: u16 = 171;
pub const COL_MFD_TYRE_PRESSURE: u16 = 172;
pub const COL_IDEAL_LINE_ON: u16 = 173;
pub const COL_IS_SETUP_MENU_VISIBLE: u16 = 174;
pub const COL_MAIN_DISPLAY_INDEX: u16 = 175;
pub const COL_SECONDARY_DISPLAY_INDEX: u16 = 176;
pub const COL_DIRECTION_LIGHTS_LEFT: u16 = 177;
pub const COL_DIRECTION_LIGHTS_RIGHT: u16 = 178;
pub const COL_TC_LEVEL: u16 = 179;
pub const COL_TC_CUT: u16 = 180;
pub const COL_ENGINE_MAP: u16 = 181;
pub const COL_ABS_LEVEL: u16 = 182;
pub const COL_EXHAUST_TEMPERATURE: u16 = 183;
pub const COL_FINAL_FF: u16 = 184;
pub const COL_PERFORMANCE_METER: u16 = 185;
pub const COL_KERB_VIBRATION: u16 = 186;
pub const COL_SLIP_VIBRATIONS: u16 = 187;
pub const COL_G_VIBRATIONS: u16 = 188;
pub const COL_ABS_VIBRATIONS: u16 = 189;

// Environment (0x0800)
pub const COL_AIR_DENSITY: u16 = 200;
pub const COL_AIR_TEMP: u16 = 201;
pub const COL_ROAD_TEMP: u16 = 202;
pub const COL_WIND_SPEED: u16 = 203;
pub const COL_WIND_DIRECTION: u16 = 204;
pub const COL_SURFACE_GRIP: u16 = 205;
pub const COL_RAIN_INTENSITY: u16 = 206;
pub const COL_RAIN_INTENSITY_IN_10MIN: u16 = 207;
pub const COL_RAIN_INTENSITY_IN_30MIN: u16 = 208;

// OtherCars (0x0900)
pub const COL_ACTIVE_CARS: u16 = 210;
pub const COL_PLAYER_CAR_ID: u16 = 211;
pub const COL_CAR_COORDINATES: u16 = 212;
pub const COL_CAR_ID: u16 = 213;

// ---------------------------------------------------------------------------
// ColumnSpec arrays (schema registration)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct ColumnSpec {
    pub id: u16,
    pub name: &'static str,
    pub value_type: u8,
}

impl ColumnSpec {
    pub const fn new(id: u16, name: &'static str, value_type: u8) -> Self {
        Self { id, name, value_type }
    }
}

pub const CONTROL_COLUMNS: [ColumnSpec; 12] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_PHYSICS_PACKET_ID, "physicsPacketId", TYPE_I32),
    ColumnSpec::new(COL_GRAPHICS_PACKET_ID, "graphicsPacketId", TYPE_I32),
    ColumnSpec::new(COL_SPEED_KMH, "speedKmh", TYPE_F32),
    ColumnSpec::new(COL_GAS, "gas", TYPE_F32),
    ColumnSpec::new(COL_BRAKE, "brake", TYPE_F32),
    ColumnSpec::new(COL_CLUTCH, "clutch", TYPE_F32),
    ColumnSpec::new(COL_STEER_ANGLE, "steerAngle", TYPE_F32),
    ColumnSpec::new(COL_GEAR, "gear", TYPE_I32),
    ColumnSpec::new(COL_RPMS, "rpms", TYPE_I32),
    ColumnSpec::new(COL_FUEL, "fuel", TYPE_F32),
];

pub const MOTION_COLUMNS: [ColumnSpec; 8] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_VELOCITY, "velocity", TYPE_BYTES),
    ColumnSpec::new(COL_ACC_G, "accG", TYPE_BYTES),
    ColumnSpec::new(COL_LOCAL_VELOCITY, "localVelocity", TYPE_BYTES),
    ColumnSpec::new(COL_LOCAL_ANGULAR_VEL, "localAngularVel", TYPE_BYTES),
    ColumnSpec::new(COL_HEADING, "heading", TYPE_F32),
    ColumnSpec::new(COL_PITCH, "pitch", TYPE_F32),
    // note: roll added at runtime; see MOTION_COLUMNS_EX
];
pub const MOTION_COLUMNS_EX: [ColumnSpec; 9] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_VELOCITY, "velocity", TYPE_BYTES),
    ColumnSpec::new(COL_ACC_G, "accG", TYPE_BYTES),
    ColumnSpec::new(COL_LOCAL_VELOCITY, "localVelocity", TYPE_BYTES),
    ColumnSpec::new(COL_LOCAL_ANGULAR_VEL, "localAngularVel", TYPE_BYTES),
    ColumnSpec::new(COL_HEADING, "heading", TYPE_F32),
    ColumnSpec::new(COL_PITCH, "pitch", TYPE_F32),
    ColumnSpec::new(COL_ROLL, "roll", TYPE_F32),
];

pub const TYRES_COLUMNS: [ColumnSpec; 31] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_WHEEL_SLIP, "wheelSlip", TYPE_BYTES),
    ColumnSpec::new(COL_WHEEL_LOAD, "wheelLoad", TYPE_BYTES),
    ColumnSpec::new(COL_WHEELS_PRESSURE, "wheelsPressure", TYPE_BYTES),
    ColumnSpec::new(COL_WHEEL_ANGULAR_SPEED, "wheelAngularSpeed", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_WEAR, "tyreWear", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_DIRTY_LEVEL, "tyreDirtyLevel", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_CORE_TEMPERATURE, "tyreCoreTemperature", TYPE_BYTES),
    ColumnSpec::new(COL_CAMBER_RAD, "camberRad", TYPE_BYTES),
    ColumnSpec::new(COL_SUSPENSION_TRAVEL, "suspensionTravel", TYPE_BYTES),
    ColumnSpec::new(COL_SLIP_RATIO, "slipRatio", TYPE_BYTES),
    ColumnSpec::new(COL_SLIP_ANGLE, "slipAngle", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_TEMP_I, "tyreTempI", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_TEMP_M, "tyreTempM", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_TEMP_O, "tyreTempO", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_TEMP, "tyreTemp", TYPE_BYTES),
    ColumnSpec::new(COL_MZ, "mz", TYPE_BYTES),
    ColumnSpec::new(COL_FX, "fx", TYPE_BYTES),
    ColumnSpec::new(COL_FY, "fy", TYPE_BYTES),
    ColumnSpec::new(COL_SUSPENSION_DAMAGE, "suspensionDamage", TYPE_BYTES),
    ColumnSpec::new(COL_BRAKE_TEMP, "brakeTemp", TYPE_BYTES),
    ColumnSpec::new(COL_BRAKE_PRESSURE, "brakePressure", TYPE_BYTES),
    ColumnSpec::new(COL_PAD_LIFE, "padLife", TYPE_BYTES),
    ColumnSpec::new(COL_DISC_LIFE, "discLife", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_CONTACT_POINT, "tyreContactPoint", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_CONTACT_NORMAL, "tyreContactNormal", TYPE_BYTES),
    ColumnSpec::new(COL_TYRE_CONTACT_HEADING, "tyreContactHeading", TYPE_BYTES),
    ColumnSpec::new(COL_NUMBER_OF_TYRES_OUT, "numberOfTyresOut", TYPE_I32),
    ColumnSpec::new(COL_FRONT_BRAKE_COMPOUND, "frontBrakeCompound", TYPE_I32),
    ColumnSpec::new(COL_REAR_BRAKE_COMPOUND, "rearBrakeCompound", TYPE_I32),
];

pub const POWERTRAIN_COLUMNS: [ColumnSpec; 24] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_TURBO_BOOST, "turboBoost", TYPE_F32),
    ColumnSpec::new(COL_BALLAST, "ballast", TYPE_F32),
    ColumnSpec::new(COL_KERS_CHARGE, "kersCharge", TYPE_F32),
    ColumnSpec::new(COL_KERS_INPUT, "kersInput", TYPE_F32),
    ColumnSpec::new(COL_KERS_CURRENT_KJ, "kersCurrentKj", TYPE_F32),
    ColumnSpec::new(COL_DRS, "drs", TYPE_F32),
    ColumnSpec::new(COL_TC_PHYSICS, "tcPhysics", TYPE_F32),
    ColumnSpec::new(COL_ABS_PHYSICS, "absPhysics", TYPE_F32),
    ColumnSpec::new(COL_ENGINE_BRAKE, "engineBrake", TYPE_I32),
    ColumnSpec::new(COL_ERS_RECOVERY_LEVEL, "ersRecoveryLevel", TYPE_I32),
    ColumnSpec::new(COL_ERS_POWER_LEVEL, "ersPowerLevel", TYPE_I32),
    ColumnSpec::new(COL_ERS_HEAT_CHARGING, "ersHeatCharging", TYPE_I32),
    ColumnSpec::new(COL_ERS_IS_CHARGING, "ersIsCharging", TYPE_I32),
    ColumnSpec::new(COL_DRS_AVAILABLE, "drsAvailable", TYPE_I32),
    ColumnSpec::new(COL_DRS_ENABLED, "drsEnabled", TYPE_I32),
    ColumnSpec::new(COL_TC_IN_ACTION, "tcInAction", TYPE_I32),
    ColumnSpec::new(COL_ABS_IN_ACTION, "absInAction", TYPE_I32),
    ColumnSpec::new(COL_AUTO_SHIFTER_ON, "autoShifterOn", TYPE_I32),
    ColumnSpec::new(COL_CURRENT_MAX_RPM, "currentMaxRpm", TYPE_I32),
    ColumnSpec::new(COL_P2P_ACTIVATIONS, "p2pActivations", TYPE_I32),
    ColumnSpec::new(COL_P2P_STATUS, "p2pStatus", TYPE_I32),
    ColumnSpec::new(COL_WATER_TEMP, "waterTemp", TYPE_F32),
];

pub const SESSION_COLUMNS: [ColumnSpec; 30] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_STATUS, "status", TYPE_I32),
    ColumnSpec::new(COL_SESSION, "session", TYPE_I32),
    ColumnSpec::new(COL_SESSION_INDEX, "sessionIndex", TYPE_I32),
    ColumnSpec::new(COL_COMPLETED_LAPS, "completedLaps", TYPE_I32),
    ColumnSpec::new(COL_POSITION, "position", TYPE_I32),
    ColumnSpec::new(COL_SESSION_TIME_LEFT, "sessionTimeLeft", TYPE_F32),
    ColumnSpec::new(COL_NUMBER_OF_LAPS, "numberOfLaps", TYPE_I32),
    ColumnSpec::new(COL_CURRENT_SECTOR_INDEX, "currentSectorIndex", TYPE_I32),
    ColumnSpec::new(COL_NORMALIZED_CAR_POSITION, "normalizedCarPosition", TYPE_F32),
    ColumnSpec::new(COL_IS_IN_PIT, "isInPit", TYPE_I32),
    ColumnSpec::new(COL_IS_IN_PIT_LANE, "isInPitLane", TYPE_I32),
    ColumnSpec::new(COL_MANDATORY_PIT_DONE, "mandatoryPitDone", TYPE_I32),
    ColumnSpec::new(COL_MISSING_MANDATORY_PITS, "missingMandatoryPits", TYPE_I32),
    ColumnSpec::new(COL_PENALTY_TIME, "penaltyTime", TYPE_F32),
    ColumnSpec::new(COL_PENALTY_TYPE, "penaltyType", TYPE_I32),
    ColumnSpec::new(COL_TRACK_STATUS, "trackStatus", TYPE_BYTES),
    ColumnSpec::new(COL_CLOCK, "clock", TYPE_F32),
    ColumnSpec::new(COL_REPLAY_TIME_MULTIPLIER, "replayTimeMultiplier", TYPE_F32),
    ColumnSpec::new(COL_IS_VALID_LAP, "isValidLap", TYPE_I32),
    ColumnSpec::new(COL_GLOBAL_YELLOW, "globalYellow", TYPE_I32),
    ColumnSpec::new(COL_GLOBAL_YELLOW1, "globalYellow1", TYPE_I32),
    ColumnSpec::new(COL_GLOBAL_YELLOW2, "globalYellow2", TYPE_I32),
    ColumnSpec::new(COL_GLOBAL_YELLOW3, "globalYellow3", TYPE_I32),
    ColumnSpec::new(COL_GLOBAL_WHITE, "globalWhite", TYPE_I32),
    ColumnSpec::new(COL_GLOBAL_GREEN, "globalGreen", TYPE_I32),
    ColumnSpec::new(COL_GLOBAL_CHEQUERED, "globalChequered", TYPE_I32),
    ColumnSpec::new(COL_GLOBAL_RED, "globalRed", TYPE_I32),
    ColumnSpec::new(COL_GAP_AHEAD_OR_TAIL, "gapAheadOrTailValue", TYPE_I32),
];

pub const TIMING_COLUMNS: [ColumnSpec; 21] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_I_CURRENT_TIME, "iCurrentTime", TYPE_I32),
    ColumnSpec::new(COL_I_LAST_TIME, "iLastTime", TYPE_I32),
    ColumnSpec::new(COL_I_BEST_TIME, "iBestTime", TYPE_I32),
    ColumnSpec::new(COL_I_SPLIT, "iSplit", TYPE_I32),
    ColumnSpec::new(COL_LAST_SECTOR_TIME, "lastSectorTime", TYPE_I32),
    ColumnSpec::new(COL_I_DELTA_LAP_TIME, "iDeltaLapTime", TYPE_I32),
    ColumnSpec::new(COL_IS_DELTA_POSITIVE, "isDeltaPositive", TYPE_I32),
    ColumnSpec::new(COL_I_ESTIMATED_LAP_TIME, "iEstimatedLapTime", TYPE_I32),
    ColumnSpec::new(COL_FUEL_ESTIMATED_LAPS, "fuelEstimatedLaps", TYPE_F32),
    ColumnSpec::new(COL_FUEL_X_LAP, "fuelXLap", TYPE_F32),
    ColumnSpec::new(COL_USED_FUEL, "usedFuel", TYPE_F32),
    ColumnSpec::new(COL_DISTANCE_TRAVELED, "distanceTraveled", TYPE_F32),
    ColumnSpec::new(COL_CURRENT_TIME_STR, "currentTimeStr", TYPE_BYTES),
    ColumnSpec::new(COL_LAST_TIME_STR, "lastTimeStr", TYPE_BYTES),
    ColumnSpec::new(COL_BEST_TIME_STR, "bestTimeStr", TYPE_BYTES),
    ColumnSpec::new(COL_SPLIT_STR, "splitStr", TYPE_BYTES),
    ColumnSpec::new(COL_DELTA_LAP_TIME_STR, "deltaLapTimeStr", TYPE_BYTES),
    ColumnSpec::new(COL_ESTIMATED_LAP_TIME_STR, "estimatedLapTimeStr", TYPE_BYTES),
    ColumnSpec::new(COL_OBSERVED_SLOT_BEFORE_I_SPLIT, "observedSlotBeforeISplit", TYPE_I32),
];

pub const CAR_STATE_COLUMNS: [ColumnSpec; 42] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_CAR_DAMAGE, "carDamage", TYPE_BYTES),
    ColumnSpec::new(COL_PIT_LIMITER_ON, "pitLimiterOn", TYPE_I32),
    ColumnSpec::new(COL_RIDE_HEIGHT, "rideHeight", TYPE_BYTES),
    ColumnSpec::new(COL_IGNITION_ON, "ignitionOn", TYPE_I32),
    ColumnSpec::new(COL_STARTER_ENGINE_ON, "starterEngineOn", TYPE_I32),
    ColumnSpec::new(COL_IS_ENGINE_RUNNING, "isEngineRunning", TYPE_I32),
    ColumnSpec::new(COL_IS_AI_CONTROLLED, "isAiControlled", TYPE_I32),
    ColumnSpec::new(COL_CG_HEIGHT, "cgHeight", TYPE_F32),
    ColumnSpec::new(COL_BRAKE_BIAS, "brakeBias", TYPE_F32),
    ColumnSpec::new(COL_RAIN_LIGHTS, "rainLights", TYPE_I32),
    ColumnSpec::new(COL_FLASHING_LIGHTS, "flashingLights", TYPE_I32),
    ColumnSpec::new(COL_LIGHTS_STAGE, "lightsStage", TYPE_I32),
    ColumnSpec::new(COL_WIPER_LV, "wiperLv", TYPE_I32),
    ColumnSpec::new(COL_DRIVER_STINT_TOTAL_TIME_LEFT, "driverStintTotalTimeLeft", TYPE_I32),
    ColumnSpec::new(COL_DRIVER_STINT_TIME_LEFT, "driverStintTimeLeft", TYPE_I32),
    ColumnSpec::new(COL_RAIN_TYRES, "rainTyres", TYPE_I32),
    ColumnSpec::new(COL_CURRENT_TYRE_SET, "currentTyreSet", TYPE_I32),
    ColumnSpec::new(COL_STRATEGY_TYRE_SET, "strategyTyreSet", TYPE_I32),
    ColumnSpec::new(COL_TRACK_GRIP_STATUS, "trackGripStatus", TYPE_I32),
    ColumnSpec::new(COL_TYRE_COMPOUND_STR, "tyreCompoundStr", TYPE_BYTES),
    ColumnSpec::new(COL_MFD_TYRE_SET, "mfdTyreSet", TYPE_I32),
    ColumnSpec::new(COL_MFD_FUEL_TO_ADD, "mfdFuelToAdd", TYPE_F32),
    ColumnSpec::new(COL_MFD_TYRE_PRESSURE, "mfdTyrePressure", TYPE_BYTES),
    ColumnSpec::new(COL_IDEAL_LINE_ON, "idealLineOn", TYPE_I32),
    ColumnSpec::new(COL_IS_SETUP_MENU_VISIBLE, "isSetupMenuVisible", TYPE_I32),
    ColumnSpec::new(COL_MAIN_DISPLAY_INDEX, "mainDisplayIndex", TYPE_I32),
    ColumnSpec::new(COL_SECONDARY_DISPLAY_INDEX, "secondaryDisplayIndex", TYPE_I32),
    ColumnSpec::new(COL_DIRECTION_LIGHTS_LEFT, "directionLightsLeft", TYPE_I32),
    ColumnSpec::new(COL_DIRECTION_LIGHTS_RIGHT, "directionLightsRight", TYPE_I32),
    ColumnSpec::new(COL_TC_LEVEL, "tcLevel", TYPE_I32),
    ColumnSpec::new(COL_TC_CUT, "tcCut", TYPE_I32),
    ColumnSpec::new(COL_ENGINE_MAP, "engineMap", TYPE_I32),
    ColumnSpec::new(COL_ABS_LEVEL, "absLevel", TYPE_I32),
    ColumnSpec::new(COL_EXHAUST_TEMPERATURE, "exhaustTemperature", TYPE_F32),
    ColumnSpec::new(COL_FINAL_FF, "finalFf", TYPE_F32),
    ColumnSpec::new(COL_PERFORMANCE_METER, "performanceMeter", TYPE_F32),
    ColumnSpec::new(COL_KERB_VIBRATION, "kerbVibration", TYPE_F32),
    ColumnSpec::new(COL_SLIP_VIBRATIONS, "slipVibrations", TYPE_F32),
    ColumnSpec::new(COL_G_VIBRATIONS, "gVibrations", TYPE_F32),
    ColumnSpec::new(COL_ABS_VIBRATIONS, "absVibrations", TYPE_F32),
];

pub const ENVIRONMENT_COLUMNS: [ColumnSpec; 11] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_AIR_DENSITY, "airDensity", TYPE_F32),
    ColumnSpec::new(COL_AIR_TEMP, "airTemp", TYPE_F32),
    ColumnSpec::new(COL_ROAD_TEMP, "roadTemp", TYPE_F32),
    ColumnSpec::new(COL_WIND_SPEED, "windSpeed", TYPE_F32),
    ColumnSpec::new(COL_WIND_DIRECTION, "windDirection", TYPE_F32),
    ColumnSpec::new(COL_SURFACE_GRIP, "surfaceGrip", TYPE_F32),
    ColumnSpec::new(COL_RAIN_INTENSITY, "rainIntensity", TYPE_I32),
    ColumnSpec::new(COL_RAIN_INTENSITY_IN_10MIN, "rainIntensityIn10min", TYPE_I32),
    ColumnSpec::new(COL_RAIN_INTENSITY_IN_30MIN, "rainIntensityIn30min", TYPE_I32),
];

pub const OTHER_CARS_COLUMNS: [ColumnSpec; 6] = [
    ColumnSpec::new(COL_SAMPLE_TICK, "sampleTick", TYPE_U64),
    ColumnSpec::new(COL_TIMESTAMP_NS, "timestampNs", TYPE_U64),
    ColumnSpec::new(COL_ACTIVE_CARS, "activeCars", TYPE_I32),
    ColumnSpec::new(COL_PLAYER_CAR_ID, "playerCarId", TYPE_I32),
    ColumnSpec::new(COL_CAR_COORDINATES, "carCoordinates", TYPE_BYTES),
    ColumnSpec::new(COL_CAR_ID, "carId", TYPE_BYTES),
];



// ---------------------------------------------------------------------------
// FileHeader, ChunkHeader, ColumnEntry, IndexEntry (unchanged)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct FileHeader {
    pub version: u16,
    pub flags: u32,
    pub schema_offset: u64,
    pub metadata_offset: u64,
    pub first_chunk_offset: u64,
    pub footer_offset: u64,
    pub created_unix_ns: u64,
    pub timebase_hz: u32,
    pub poll_hz_x1000: u32,
}

impl FileHeader {
    pub fn new(created_unix_ns: u64, poll_hz: f64) -> Self {
        Self {
            version: FORMAT_VERSION,
            flags: 0,
            schema_offset: HEADER_SIZE as u64,
            metadata_offset: 0,
            first_chunk_offset: 0,
            footer_offset: 0,
            created_unix_ns,
            timebase_hz: TIMEBASE_HZ,
            poll_hz_x1000: hz_to_x1000(poll_hz),
        }
    }

    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&MAGIC)?;
        writer.write_all(&self.version.to_le_bytes())?;
        writer.write_all(&HEADER_SIZE.to_le_bytes())?;
        writer.write_all(&self.flags.to_le_bytes())?;
        writer.write_all(&self.schema_offset.to_le_bytes())?;
        writer.write_all(&self.metadata_offset.to_le_bytes())?;
        writer.write_all(&self.first_chunk_offset.to_le_bytes())?;
        writer.write_all(&self.footer_offset.to_le_bytes())?;
        writer.write_all(&self.created_unix_ns.to_le_bytes())?;
        writer.write_all(&self.timebase_hz.to_le_bytes())?;
        writer.write_all(&self.poll_hz_x1000.to_le_bytes())?;
        writer.write_all(&[0u8; HEADER_SIZE as usize - 64])?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if magic != MAGIC {
            return Err(TelemetryError::InvalidFormat("bad ACTL magic".to_string()));
        }

        let version = read_u16(reader)?;
        if version != 1 && version != FORMAT_VERSION {
            return Err(TelemetryError::UnsupportedVersion(version));
        }

        let header_size = read_u16(reader)?;
        if header_size != HEADER_SIZE {
            return Err(TelemetryError::InvalidFormat(format!(
                "unexpected header size {header_size}"
            )));
        }

        let flags = read_u32(reader)?;
        let schema_offset = read_u64(reader)?;
        let metadata_offset = read_u64(reader)?;
        let first_chunk_offset = read_u64(reader)?;
        let footer_offset = read_u64(reader)?;
        let created_unix_ns = read_u64(reader)?;
        let timebase_hz = read_u32(reader)?;
        let poll_hz_x1000 = read_u32(reader)?;
        let mut reserved = vec![0u8; HEADER_SIZE as usize - 64];
        reader.read_exact(&mut reserved)?;

        Ok(Self {
            version,
            flags,
            schema_offset,
            metadata_offset,
            first_chunk_offset,
            footer_offset,
            created_unix_ns,
            timebase_hz,
            poll_hz_x1000,
        })
    }

    pub fn poll_hz(&self) -> f64 {
        x1000_to_hz(self.poll_hz_x1000)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkHeader {
    pub cluster_id: u16,
    pub chunk_seq: u32,
    pub schema_hash: u64,
    pub base_sample_tick: u64,
    pub sample_stride: u32,
    pub sample_count: u32,
    pub start_time_ns: u64,
    pub end_time_ns: u64,
    pub start_lap: i32,
    pub end_lap: i32,
    pub column_count: u16,
    pub flags: u16,
    pub payload_len: u32,
    pub payload_crc32: u32,
}

impl ChunkHeader {
    pub fn byte_len(column_count: usize) -> usize {
        CHUNK_HEADER_SIZE as usize + column_count * COLUMN_ENTRY_SIZE
    }

    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&CHUNK_MAGIC)?;
        writer.write_all(&CHUNK_HEADER_SIZE.to_le_bytes())?;
        writer.write_all(&self.cluster_id.to_le_bytes())?;
        writer.write_all(&self.chunk_seq.to_le_bytes())?;
        writer.write_all(&self.schema_hash.to_le_bytes())?;
        writer.write_all(&self.base_sample_tick.to_le_bytes())?;
        writer.write_all(&self.sample_stride.to_le_bytes())?;
        writer.write_all(&self.sample_count.to_le_bytes())?;
        writer.write_all(&self.start_time_ns.to_le_bytes())?;
        writer.write_all(&self.end_time_ns.to_le_bytes())?;
        writer.write_all(&self.start_lap.to_le_bytes())?;
        writer.write_all(&self.end_lap.to_le_bytes())?;
        writer.write_all(&self.column_count.to_le_bytes())?;
        writer.write_all(&self.flags.to_le_bytes())?;
        writer.write_all(&self.payload_len.to_le_bytes())?;
        writer.write_all(&self.payload_crc32.to_le_bytes())?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != CHUNK_MAGIC {
            return Err(TelemetryError::InvalidFormat("bad chunk magic".to_string()));
        }
        let header_size = read_u16(reader)?;
        if header_size != CHUNK_HEADER_SIZE {
            return Err(TelemetryError::InvalidFormat(format!(
                "unexpected chunk header size {header_size}"
            )));
        }
        Ok(Self {
            cluster_id: read_u16(reader)?,
            chunk_seq: read_u32(reader)?,
            schema_hash: read_u64(reader)?,
            base_sample_tick: read_u64(reader)?,
            sample_stride: read_u32(reader)?,
            sample_count: read_u32(reader)?,
            start_time_ns: read_u64(reader)?,
            end_time_ns: read_u64(reader)?,
            start_lap: read_i32(reader)?,
            end_lap: read_i32(reader)?,
            column_count: read_u16(reader)?,
            flags: read_u16(reader)?,
            payload_len: read_u32(reader)?,
            payload_crc32: read_u32(reader)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ColumnEntry {
    pub column_id: u16,
    pub codec: u8,
    pub value_type: u8,
    pub lane_count: u8,
    pub flags: u8,
    pub offset: u32,
    pub byte_len: u32,
    pub null_offset: u32,
    pub min_value: f64,
    pub max_value: f64,
}

impl ColumnEntry {
    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&self.column_id.to_le_bytes())?;
        writer.write_all(&[self.codec, self.value_type, self.lane_count, self.flags])?;
        writer.write_all(&self.offset.to_le_bytes())?;
        writer.write_all(&self.byte_len.to_le_bytes())?;
        writer.write_all(&self.null_offset.to_le_bytes())?;
        writer.write_all(&self.min_value.to_le_bytes())?;
        writer.write_all(&self.max_value.to_le_bytes())?;
        writer.write_all(&[0u8; 6])?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let column_id = read_u16(reader)?;
        let mut meta = [0u8; 4];
        reader.read_exact(&mut meta)?;
        let offset = read_u32(reader)?;
        let byte_len = read_u32(reader)?;
        let null_offset = read_u32(reader)?;
        let min_value = read_f64(reader)?;
        let max_value = read_f64(reader)?;
        let mut reserved = [0u8; 6];
        reader.read_exact(&mut reserved)?;
        Ok(Self {
            column_id,
            codec: meta[0],
            value_type: meta[1],
            lane_count: meta[2],
            flags: meta[3],
            offset,
            byte_len,
            null_offset,
            min_value,
            max_value,
        })
    }
}

#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub cluster_id: u16,
    pub chunk_seq: u32,
    pub file_offset: u64,
    pub byte_len: u32,
    pub start_time_ns: u64,
    pub end_time_ns: u64,
    pub start_tick: u64,
    pub end_tick: u64,
}

impl IndexEntry {
    pub const BYTE_LEN: usize = 56;

    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&self.cluster_id.to_le_bytes())?;
        writer.write_all(&[0u8; 2])?;
        writer.write_all(&self.chunk_seq.to_le_bytes())?;
        writer.write_all(&self.file_offset.to_le_bytes())?;
        writer.write_all(&self.byte_len.to_le_bytes())?;
        writer.write_all(&[0u8; 4])?;
        writer.write_all(&self.start_time_ns.to_le_bytes())?;
        writer.write_all(&self.end_time_ns.to_le_bytes())?;
        writer.write_all(&self.start_tick.to_le_bytes())?;
        writer.write_all(&self.end_tick.to_le_bytes())?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let cluster_id = read_u16(reader)?;
        let _reserved0 = read_u16(reader)?;
        let chunk_seq = read_u32(reader)?;
        let file_offset = read_u64(reader)?;
        let byte_len = read_u32(reader)?;
        let _reserved1 = read_u32(reader)?;
        let start_time_ns = read_u64(reader)?;
        let end_time_ns = read_u64(reader)?;
        let start_tick = read_u64(reader)?;
        let end_tick = read_u64(reader)?;
        Ok(Self {
            cluster_id,
            chunk_seq,
            file_offset,
            byte_len,
            start_time_ns,
            end_time_ns,
            start_tick,
            end_tick,
        })
    }
}

// ---------------------------------------------------------------------------
// Schema encoding (registers all clusters)
// ---------------------------------------------------------------------------

pub fn encode_schema() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&SCHEMA_MAGIC);
    out.extend_from_slice(&SCHEMA_HASH.to_le_bytes());
    out.extend_from_slice(&9u16.to_le_bytes()); // 9 clusters

    write_schema_cluster(&mut out, CLUSTER_CONTROLS, &CONTROL_COLUMNS);
    write_schema_cluster(&mut out, CLUSTER_MOTION, &MOTION_COLUMNS);
    write_schema_cluster(&mut out, CLUSTER_TYRES, &TYRES_COLUMNS);
    write_schema_cluster(&mut out, CLUSTER_POWERTRAIN, &POWERTRAIN_COLUMNS);
    write_schema_cluster(&mut out, CLUSTER_SESSION, &SESSION_COLUMNS);
    write_schema_cluster(&mut out, CLUSTER_TIMING, &TIMING_COLUMNS);
    write_schema_cluster(&mut out, CLUSTER_CAR_STATE, &CAR_STATE_COLUMNS);
    write_schema_cluster(&mut out, CLUSTER_ENVIRONMENT, &ENVIRONMENT_COLUMNS);
    write_schema_cluster(&mut out, CLUSTER_OTHER_CARS, &OTHER_CARS_COLUMNS);
    out
}

fn write_schema_cluster(out: &mut Vec<u8>, cluster_id: u16, columns: &[ColumnSpec]) {
    out.extend_from_slice(&cluster_id.to_le_bytes());
    out.extend_from_slice(&(columns.len() as u16).to_le_bytes());
    for column in columns {
        out.extend_from_slice(&column.id.to_le_bytes());
        out.push(column.value_type);
        out.push(column.name.len() as u8);
        out.extend_from_slice(column.name.as_bytes());
    }
}

pub fn validate_schema(bytes: &[u8]) -> TelemetryResult<()> {
    if bytes.len() < 14 || bytes[0..4] != SCHEMA_MAGIC {
        return Err(TelemetryError::InvalidFormat("bad schema block".to_string()));
    }
    let hash = u64::from_le_bytes(bytes[4..12].try_into().unwrap());
    if hash != SCHEMA_HASH {
        return Err(TelemetryError::InvalidFormat("schema hash mismatch".to_string()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper functions (unchanged)
// ---------------------------------------------------------------------------

pub fn hz_to_x1000(hz: f64) -> u32 {
    if hz.is_finite() && hz > 0.0 {
        (hz * 1000.0).round().clamp(1.0, u32::MAX as f64) as u32
    } else {
        120_000
    }
}

pub fn x1000_to_hz(value: u32) -> f64 {
    value as f64 / 1000.0
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

pub fn read_u16(reader: &mut impl Read) -> TelemetryResult<u16> {
    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

pub fn read_u32(reader: &mut impl Read) -> TelemetryResult<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

pub fn read_i32(reader: &mut impl Read) -> TelemetryResult<i32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(i32::from_le_bytes(buf))
}

pub fn read_u64(reader: &mut impl Read) -> TelemetryResult<u64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

pub fn read_f64(reader: &mut impl Read) -> TelemetryResult<f64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(f64::from_le_bytes(buf))
}
