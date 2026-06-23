//! Format V2: Logical access groups and column catalog
//!
//! This module defines the v2 format's column grouping strategy,
//! assigning all unique telemetry columns to 7 logical access groups.
//!
//! # Design Principles
//!
//! - **Deduplicated**: Columns like `sample_tick` and `timestamp_ns` that
//!   appear in every v1 cluster are defined once in `frame_meta`.
//! - **Access-pattern aligned**: Columns commonly read together in racing
//!   telemetry analysis reside in the same group, enabling efficient
//!   columnar storage and I/O.
//! - **7 groups**: 3 hot groups (frame_meta, driver_inputs, timing),
//!   2 warm groups (vehicle_dynamics, tyres), 1 cool group (environment),
//!   1 cold group (cold_storage) — reflecting real-world query frequency.
//!
//! # Groups Overview
//!
//! | Group | Columns | Access Pattern |
//! |-------|---------|----------------|
//! | FrameMeta | 4 | Always read first (frame identity) |
//! | DriverInputs | 22 | Hot — every tick, driver telemetry |
//! | VehicleDynamics | 7 | Warm — motion analysis queries |
//! | Tyres | 29 | Warm — tyre degradation analysis |
//! | Timing | 26 | Hot — lap/sector comparison |
//! | Environment | 15 | Cool — session conditions |
//! | ColdStorage | 61 | Rare — setup, other cars, flags |

// ---------------------------------------------------------------------------
// GroupId — 7 logical access groups
// ---------------------------------------------------------------------------

/// Logical access group for telemetry columns.
///
/// Each group collects columns that are typically read together during
/// racing telemetry analysis, enabling efficient columnar I/O paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupId {
    /// Frame identity metadata — `sample_tick`, `timestamp_ns`,
    /// `physics_packet_id`, `graphics_packet_id`.
    ///
    /// Always read first to establish the frame context.
    /// These 4 columns appear in every v1 cluster but are stored once here.
    FrameMeta,

    /// Driver inputs and powertrain state — controls (speed, gas, brake,
    /// clutch, steer, gear, rpms, fuel) plus all powertrain/ERS fields.
    ///
    /// Hot path: queried on every tick for driving telemetry dashboards
    /// and performance analysis.
    DriverInputs,

    /// Vehicle motion and dynamics — velocity, acceleration, heading,
    /// pitch, roll, and angular velocity.
    ///
    /// Warm path: queried during vehicle dynamics analysis, trajectory
    /// reconstruction, and incident review.
    VehicleDynamics,

    /// Tyre and wheel data — temperatures, pressures, wear, slip, loads,
    /// suspension travel, brake temperatures, and contact patch info.
    ///
    /// Warm path: tyre degradation studies, setup analysis, and
    /// grip assessment.
    Tyres,

    /// Lap/sector timing and session position — lap times, sector splits,
    /// deltas, fuel estimates, and session position/lap count.
    ///
    /// Hot path: queried alongside frame_meta for live timing displays
    /// and post-session lap analysis.
    Timing,

    /// Environmental and session conditions — air/track temperature,
    /// wind, rain intensity, surface grip, session type/status, and
    /// clock/replay info.
    ///
    /// Cool path: read at session boundaries or when context changes.
    Environment,

    /// Rarely-accessed fields — car setup/settings, car damage/state
    /// details, flag status, OtherCars, and administrative session fields.
    ///
    /// Cold path: only queried during deep analysis, incident
    /// reconstruction, or setup comparison.
    ColdStorage,
}

// ---------------------------------------------------------------------------
// ColumnId — unique identifier for every raw telemetry column
// ---------------------------------------------------------------------------

/// Unique identifier for every column stored in acctlm2 v2 format.
///
/// Values correspond 1:1 with the v1 `COL_*` constants in `format.rs`,
/// but `sample_tick` and `timestamp_ns` are defined only once (in
/// `frame_meta`), unlike v1 where they repeat in every cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ColumnId {
    // -- frame_meta (IDs 1-4) --
    SampleTick = 1,
    TimestampNs = 2,
    PhysicsPacketId = 3,
    GraphicsPacketId = 4,

    // -- driver_inputs | Controls (IDs 10-17) --
    SpeedKmh = 10,
    Gas = 11,
    Brake = 12,
    Clutch = 13,
    SteerAngle = 14,
    Gear = 15,
    Rpms = 16,
    Fuel = 17,

    // -- vehicle_dynamics | Motion (IDs 20-26) --
    Velocity = 20,
    AccG = 21,
    LocalVelocity = 22,
    LocalAngularVel = 23,
    Heading = 24,
    Pitch = 25,
    Roll = 26,

    // -- tyres | Wheels & Tyres (IDs 30-58) --
    WheelSlip = 30,
    WheelLoad = 31,
    WheelsPressure = 32,
    WheelAngularSpeed = 33,
    TyreWear = 34,
    TyreDirtyLevel = 35,
    TyreCoreTemperature = 36,
    CamberRad = 37,
    SuspensionTravel = 38,
    SlipRatio = 39,
    SlipAngle = 40,
    TyreTempI = 41,
    TyreTempM = 42,
    TyreTempO = 43,
    TyreTemp = 44,
    Mz = 45,
    Fx = 46,
    Fy = 47,
    SuspensionDamage = 48,
    BrakeTemp = 49,
    BrakePressure = 50,
    PadLife = 51,
    DiscLife = 52,
    TyreContactPoint = 53,
    TyreContactNormal = 54,
    TyreContactHeading = 55,
    NumberOfTyresOut = 56,
    FrontBrakeCompound = 57,
    RearBrakeCompound = 58,

    // -- driver_inputs | Powertrain (IDs 60-81) --
    TurboBoost = 60,
    Ballast = 61,
    KersCharge = 62,
    KersInput = 63,
    KersCurrentKj = 64,
    Drs = 65,
    TcPhysics = 66,
    AbsPhysics = 67,
    EngineBrake = 68,
    ErsRecoveryLevel = 69,
    ErsPowerLevel = 70,
    ErsHeatCharging = 71,
    ErsIsCharging = 72,
    DrsAvailable = 73,
    DrsEnabled = 74,
    TcInAction = 75,
    AbsInAction = 76,
    AutoShifterOn = 77,
    CurrentMaxRpm = 78,
    P2pActivations = 79,
    P2pStatus = 80,
    WaterTemp = 81,

    // -- timing + environment | Session shared (IDs 90-119) --
    Status = 90,
    Session = 91,
    SessionIndex = 92,
    CompletedLaps = 93,
    Position = 94,
    SessionTimeLeft = 95,
    NumberOfLaps = 96,
    CurrentSectorIndex = 97,
    NormalizedCarPosition = 98,
    IsInPit = 99,
    IsInPitLane = 100,
    MandatoryPitDone = 101,
    MissingMandatoryPits = 102,
    PenaltyTime = 103,
    PenaltyType = 104,
    TrackStatus = 105,
    Clock = 106,
    ReplayTimeMultiplier = 107,
    IsValidLap = 108,
    GlobalYellow = 109,
    GlobalYellow1 = 110,
    GlobalYellow2 = 111,
    GlobalYellow3 = 112,
    GlobalWhite = 113,
    GlobalGreen = 114,
    GlobalChequered = 115,
    GlobalRed = 116,
    GapAheadOrTailValue = 117,
    Flag = 118,
    GapBehind = 119,

    // -- timing | Timing specific (IDs 120-138) --
    ICurrentTime = 120,
    ILastTime = 121,
    IBestTime = 122,
    ISplit = 123,
    LastSectorTime = 124,
    IDeltaLapTime = 125,
    IsDeltaPositive = 126,
    IEstimatedLapTime = 127,
    FuelEstimatedLaps = 128,
    FuelXLap = 129,
    UsedFuel = 130,
    DistanceTraveled = 131,
    CurrentTimeStr = 132,
    LastTimeStr = 133,
    BestTimeStr = 134,
    SplitStr = 135,
    DeltaLapTimeStr = 136,
    EstimatedLapTimeStr = 137,
    ObservedSlotBeforeISplit = 138,

    // -- cold_storage | CarState (IDs 150-189) --
    CarDamage = 150,
    PitLimiterOn = 151,
    RideHeight = 152,
    IgnitionOn = 153,
    StarterEngineOn = 154,
    IsEngineRunning = 155,
    IsAiControlled = 156,
    CgHeight = 157,
    BrakeBias = 158,
    RainLights = 159,
    FlashingLights = 160,
    LightsStage = 161,
    WiperLv = 162,
    DriverStintTotalTimeLeft = 163,
    DriverStintTimeLeft = 164,
    RainTyres = 165,
    CurrentTyreSet = 166,
    StrategyTyreSet = 167,
    TrackGripStatus = 168,
    TyreCompoundStr = 169,
    MfdTyreSet = 170,
    MfdFuelToAdd = 171,
    MfdTyrePressure = 172,
    IdealLineOn = 173,
    IsSetupMenuVisible = 174,
    MainDisplayIndex = 175,
    SecondaryDisplayIndex = 176,
    DirectionLightsLeft = 177,
    DirectionLightsRight = 178,
    TcLevel = 179,
    TcCut = 180,
    EngineMap = 181,
    AbsLevel = 182,
    ExhaustTemperature = 183,
    FinalFf = 184,
    PerformanceMeter = 185,
    KerbVibration = 186,
    SlipVibrations = 187,
    GVibrations = 188,
    AbsVibrations = 189,

    // -- cold_storage | Environment (IDs 200-208, not surface/rain) --
    // (Note: AirDensity, AirTemp, RoadTemp, WindSpeed etc. are in
    //  Environment group below, using same IDs)

    // -- environment | Environment specific (IDs 200-208) --
    AirDensity = 200,
    AirTemp = 201,
    RoadTemp = 202,
    WindSpeed = 203,
    WindDirection = 204,
    SurfaceGrip = 205,
    RainIntensity = 206,
    RainIntensityIn10min = 207,
    RainIntensityIn30min = 208,

    // -- cold_storage | OtherCars (IDs 210-213) --
    ActiveCars = 210,
    PlayerCarId = 211,
    CarCoordinates = 212,
    CarId = 213,
}

// ---------------------------------------------------------------------------
// ColumnId → GroupId mapping
// ---------------------------------------------------------------------------

/// Return the logical access group for a given column.
///
/// # Example
///
/// ```
/// use module_live_telemetry::format_v2::{ColumnId, GroupId, column_group};
///
/// assert_eq!(column_group(ColumnId::SpeedKmh), GroupId::DriverInputs);
/// assert_eq!(column_group(ColumnId::SampleTick), GroupId::FrameMeta);
/// assert_eq!(column_group(ColumnId::CarDamage), GroupId::ColdStorage);
/// ```
pub const fn column_group(col: ColumnId) -> GroupId {
    // Dispatch by numeric range for clarity.
    // Each arm covers a contiguous block of IDs assigned to the same group.
    match col as u16 {
        // frame_meta: IDs 1-4
        1..=4 => GroupId::FrameMeta,

        // driver_inputs: Controls (10-17) + Powertrain (60-81)
        10..=17 => GroupId::DriverInputs,
        60..=81 => GroupId::DriverInputs,

        // vehicle_dynamics: Motion (20-26)
        20..=26 => GroupId::VehicleDynamics,

        // tyres: Wheels & Tyres (30-58)
        30..=58 => GroupId::Tyres,

        // timing: Session position/lap (93-94, 96-99, 108) + Timing specific (120-138)
        93..=94 => GroupId::Timing,
        96..=99 => GroupId::Timing,
        108 => GroupId::Timing,
        120..=138 => GroupId::Timing,

        // environment: Session status/clock (90-91, 95, 106-107) + Environment (200-208)
        90..=91 => GroupId::Environment,
        95 => GroupId::Environment,
        106..=107 => GroupId::Environment,
        200..=208 => GroupId::Environment,

        // cold_storage: remaining Session (92, 100-105, 109-119) + CarState (150-189) + OtherCars (210-213)
        92 => GroupId::ColdStorage,
        100..=105 => GroupId::ColdStorage,
        109..=119 => GroupId::ColdStorage,
        150..=189 => GroupId::ColdStorage,
        210..=213 => GroupId::ColdStorage,

        // Safety: unreachable since ColumnId is repr(u16) and all defined
        // variants are covered above.
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_meta_group() {
        assert_eq!(column_group(ColumnId::SampleTick), GroupId::FrameMeta);
        assert_eq!(column_group(ColumnId::TimestampNs), GroupId::FrameMeta);
        assert_eq!(column_group(ColumnId::PhysicsPacketId), GroupId::FrameMeta);
        assert_eq!(column_group(ColumnId::GraphicsPacketId), GroupId::FrameMeta);
    }

    #[test]
    fn test_driver_inputs_group() {
        // Controls
        assert_eq!(column_group(ColumnId::SpeedKmh), GroupId::DriverInputs);
        assert_eq!(column_group(ColumnId::Gas), GroupId::DriverInputs);
        assert_eq!(column_group(ColumnId::Brake), GroupId::DriverInputs);
        assert_eq!(column_group(ColumnId::Gear), GroupId::DriverInputs);
        // Powertrain
        assert_eq!(column_group(ColumnId::TurboBoost), GroupId::DriverInputs);
        assert_eq!(column_group(ColumnId::Drs), GroupId::DriverInputs);
        assert_eq!(column_group(ColumnId::KersCharge), GroupId::DriverInputs);
        assert_eq!(column_group(ColumnId::EngineBrake), GroupId::DriverInputs);
    }

    #[test]
    fn test_vehicle_dynamics_group() {
        assert_eq!(column_group(ColumnId::Velocity), GroupId::VehicleDynamics);
        assert_eq!(column_group(ColumnId::AccG), GroupId::VehicleDynamics);
        assert_eq!(column_group(ColumnId::Heading), GroupId::VehicleDynamics);
        assert_eq!(column_group(ColumnId::Roll), GroupId::VehicleDynamics);
    }

    #[test]
    fn test_tyres_group() {
        assert_eq!(column_group(ColumnId::WheelSlip), GroupId::Tyres);
        assert_eq!(column_group(ColumnId::TyreWear), GroupId::Tyres);
        assert_eq!(column_group(ColumnId::BrakeTemp), GroupId::Tyres);
        assert_eq!(column_group(ColumnId::NumberOfTyresOut), GroupId::Tyres);
    }

    #[test]
    fn test_timing_group() {
        // Timing specific
        assert_eq!(column_group(ColumnId::ICurrentTime), GroupId::Timing);
        assert_eq!(column_group(ColumnId::IDeltaLapTime), GroupId::Timing);
        // Session timing-related
        assert_eq!(column_group(ColumnId::Position), GroupId::Timing);
        assert_eq!(column_group(ColumnId::CompletedLaps), GroupId::Timing);
        assert_eq!(column_group(ColumnId::IsInPit), GroupId::Timing);
    }

    #[test]
    fn test_environment_group() {
        // Session status
        assert_eq!(column_group(ColumnId::Status), GroupId::Environment);
        assert_eq!(column_group(ColumnId::Session), GroupId::Environment);
        assert_eq!(
            column_group(ColumnId::SessionTimeLeft),
            GroupId::Environment
        );
        // Environment specific
        assert_eq!(column_group(ColumnId::AirTemp), GroupId::Environment);
        assert_eq!(column_group(ColumnId::RainIntensity), GroupId::Environment);
    }

    #[test]
    fn test_cold_storage_group() {
        // Session cold fields
        assert_eq!(column_group(ColumnId::IsInPitLane), GroupId::ColdStorage);
        assert_eq!(column_group(ColumnId::PenaltyTime), GroupId::ColdStorage);
        assert_eq!(column_group(ColumnId::GlobalYellow), GroupId::ColdStorage);
        // CarState
        assert_eq!(column_group(ColumnId::CarDamage), GroupId::ColdStorage);
        assert_eq!(column_group(ColumnId::CgHeight), GroupId::ColdStorage);
        assert_eq!(column_group(ColumnId::TcLevel), GroupId::ColdStorage);
        // OtherCars
        assert_eq!(column_group(ColumnId::ActiveCars), GroupId::ColdStorage);
        assert_eq!(column_group(ColumnId::CarId), GroupId::ColdStorage);
    }

    #[test]
    fn test_all_variants_resolve() {
        // Spot-check that every range in the dispatch has at least
        // one representative that maps correctly.
        let cases: &[(ColumnId, GroupId)] = &[
            // frame_meta
            (ColumnId::SampleTick, GroupId::FrameMeta),
            (ColumnId::GraphicsPacketId, GroupId::FrameMeta),
            // driver_inputs
            (ColumnId::Fuel, GroupId::DriverInputs),
            (ColumnId::WaterTemp, GroupId::DriverInputs),
            // vehicle_dynamics
            (ColumnId::LocalAngularVel, GroupId::VehicleDynamics),
            // tyres
            (ColumnId::RearBrakeCompound, GroupId::Tyres),
            // timing
            (ColumnId::ObservedSlotBeforeISplit, GroupId::Timing),
            (ColumnId::IsValidLap, GroupId::Timing),
            (ColumnId::NormalizedCarPosition, GroupId::Timing),
            // environment
            (ColumnId::RainIntensityIn30min, GroupId::Environment),
            (ColumnId::ReplayTimeMultiplier, GroupId::Environment),
            // cold_storage
            (ColumnId::AbsVibrations, GroupId::ColdStorage),
            (ColumnId::PlayerCarId, GroupId::ColdStorage),
        ];
        for &(col, expected) in cases {
            assert_eq!(column_group(col), expected, "Mismatch for {col:?}");
        }
    }

    #[test]
    fn test_column_id_repr() {
        assert_eq!(ColumnId::SampleTick as u16, 1);
        assert_eq!(ColumnId::SpeedKmh as u16, 10);
        assert_eq!(ColumnId::CarId as u16, 213);
    }

    #[test]
    fn test_group_count() {
        // Verify all 7 groups exist by collecting unique values
        let all_groups = vec![
            GroupId::FrameMeta,
            GroupId::DriverInputs,
            GroupId::VehicleDynamics,
            GroupId::Tyres,
            GroupId::Timing,
            GroupId::Environment,
            GroupId::ColdStorage,
        ];
        assert_eq!(all_groups.len(), 7);
    }
}

// ===========================================================================
// V2 Binary Format Types
// ===========================================================================

use crate::error::{TelemetryError, TelemetryResult};
use std::io::{Read, Write};

// ---------------------------------------------------------------------------
// V2 Value type constants (expanded to include f64)
// ---------------------------------------------------------------------------
/// Unsigned 64-bit integer.
pub const TYPE_U64: u8 = 0x01;
/// Signed 32-bit integer.
pub const TYPE_I32: u8 = 0x02;
/// 32-bit float.
pub const TYPE_F32: u8 = 0x03;
/// 64-bit float.
pub const TYPE_F64: u8 = 0x04;
/// Variable-length byte blob.
pub const TYPE_BYTES: u8 = 0x05;
/// Variable-length byte blob with f32 sub-values (4 bytes each).
pub const TYPE_BYTES_F32: u8 = 0x06;
/// Variable-length byte blob with u16 sub-values (2 bytes each).
pub const TYPE_BYTES_U16: u8 = 0x07;
/// Variable-length byte blob with i32 sub-values (4 bytes each).
pub const TYPE_BYTES_I32: u8 = 0x08;

// ---------------------------------------------------------------------------
// Binary format size constants
// ---------------------------------------------------------------------------
pub const HEADER_V2_SIZE: usize = 64;
pub const ROW_GROUP_HEADER_SIZE: usize = 32;
pub const COLUMN_ENTRY_V2_SIZE: usize = 40;

// ---------------------------------------------------------------------------
// Magic constants
// ---------------------------------------------------------------------------
const MAGIC_ACT2: [u8; 4] = *b"ACT2";
const MAGIC_RGHD: [u8; 4] = *b"RGHD";
const MAGIC_FTR2: [u8; 4] = *b"FTR2";

// ---------------------------------------------------------------------------
// Read helpers (private to this module)
// ---------------------------------------------------------------------------
fn read_u16(reader: &mut impl Read) -> TelemetryResult<u16> {
    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32(reader: &mut impl Read) -> TelemetryResult<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64(reader: &mut impl Read) -> TelemetryResult<u64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_f64(reader: &mut impl Read) -> TelemetryResult<f64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(f64::from_le_bytes(buf))
}

// ---------------------------------------------------------------------------
// FileHeaderV2 — 64-byte file header
// ---------------------------------------------------------------------------

/// V2 file header. Exactly 64 bytes.
///
/// Layout:
/// - magic:            b"ACT2"   (4 bytes)
/// - version:          2         (2 bytes)
/// - schema_offset:    u64       (8 bytes)
/// - metadata_offset:  u64       (8 bytes)
/// - first_row_group_offset: u64 (8 bytes)
/// - footer_offset:    u64       (8 bytes)
/// - created_unix_ns:  u64       (8 bytes)
/// - poll_hz:          u32       (4 bytes)
/// - _reserved:        [u8; 14]  (14 bytes padding)
#[derive(Debug, Clone, PartialEq)]
pub struct FileHeaderV2 {
    pub schema_offset: u64,
    pub metadata_offset: u64,
    pub first_row_group_offset: u64,
    pub footer_offset: u64,
    pub created_unix_ns: u64,
    pub poll_hz: u32,
}

impl FileHeaderV2 {
    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&MAGIC_ACT2)?;
        writer.write_all(&2u16.to_le_bytes())?;
        writer.write_all(&self.schema_offset.to_le_bytes())?;
        writer.write_all(&self.metadata_offset.to_le_bytes())?;
        writer.write_all(&self.first_row_group_offset.to_le_bytes())?;
        writer.write_all(&self.footer_offset.to_le_bytes())?;
        writer.write_all(&self.created_unix_ns.to_le_bytes())?;
        writer.write_all(&self.poll_hz.to_le_bytes())?;
        writer.write_all(&[0u8; 14])?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != MAGIC_ACT2 {
            return Err(TelemetryError::InvalidFormat(
                "bad v2 file header magic".to_string(),
            ));
        }
        let version = read_u16(reader)?;
        if version != 2 {
            return Err(TelemetryError::InvalidFormat(format!(
                "unsupported v2 format version: {version}"
            )));
        }
        let schema_offset = read_u64(reader)?;
        let metadata_offset = read_u64(reader)?;
        let first_row_group_offset = read_u64(reader)?;
        let footer_offset = read_u64(reader)?;
        let created_unix_ns = read_u64(reader)?;
        let poll_hz = read_u32(reader)?;
        let mut reserved = [0u8; 14];
        reader.read_exact(&mut reserved)?;
        Ok(Self {
            schema_offset,
            metadata_offset,
            first_row_group_offset,
            footer_offset,
            created_unix_ns,
            poll_hz,
        })
    }
}

// ---------------------------------------------------------------------------
// SchemaBlockV2 — group/column catalog
// ---------------------------------------------------------------------------

/// A single column definition within a schema group.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaColumnDefV2 {
    pub column_id: u16,
    pub value_type: u8,
    pub name: String,
}

/// A single group definition within the schema.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaGroupDefV2 {
    pub group_id: u16,
    pub columns: Vec<SchemaColumnDefV2>,
}

/// Schema block: the full column catalog organised by access group.
///
/// Layout:
/// - group_count: u16
/// - For each group:
///   - group_id: u16
///   - column_count: u16
///   - For each column:
///     - column_id: u16
///     - value_type: u8
///     - name_len: u8
///     - name: [u8; name_len]
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaBlockV2 {
    pub groups: Vec<SchemaGroupDefV2>,
}

impl SchemaBlockV2 {
    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        let group_count = self.groups.len() as u16;
        writer.write_all(&group_count.to_le_bytes())?;
        for group in &self.groups {
            writer.write_all(&group.group_id.to_le_bytes())?;
            let col_count = group.columns.len() as u16;
            writer.write_all(&col_count.to_le_bytes())?;
            for col in &group.columns {
                writer.write_all(&col.column_id.to_le_bytes())?;
                writer.write_all(&[col.value_type])?;
                let name_bytes = col.name.as_bytes();
                let name_len = name_bytes.len().min(255) as u8;
                writer.write_all(&[name_len])?;
                writer.write_all(&name_bytes[..name_len as usize])?;
            }
        }
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let group_count = read_u16(reader)?;
        let mut groups = Vec::with_capacity(group_count as usize);
        for _ in 0..group_count {
            let group_id = read_u16(reader)?;
            let col_count = read_u16(reader)?;
            let mut columns = Vec::with_capacity(col_count as usize);
            for _ in 0..col_count {
                let column_id = read_u16(reader)?;
                let mut meta = [0u8; 2];
                reader.read_exact(&mut meta)?;
                let value_type = meta[0];
                let name_len = meta[1] as usize;
                let mut name_buf = vec![0u8; name_len];
                reader.read_exact(&mut name_buf)?;
                let name = String::from_utf8(name_buf).map_err(|e| {
                    TelemetryError::InvalidFormat(format!("invalid column name: {e}"))
                })?;
                columns.push(SchemaColumnDefV2 {
                    column_id,
                    value_type,
                    name,
                });
            }
            groups.push(SchemaGroupDefV2 { group_id, columns });
        }
        Ok(Self { groups })
    }
}

// ---------------------------------------------------------------------------
// RowGroupHeader — per-row-group metadata
// ---------------------------------------------------------------------------

/// Per-group offset/size pair within a row group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroupEntryV2 {
    pub group_id: u16,
    pub offset: u32,
    pub byte_len: u32,
}

/// Row group header (variable-length).
///
/// Fixed portion (32 bytes):
/// - magic:            b"RGHD"   (4 bytes)
/// - row_count:        u32       (4 bytes)
/// - frame_start_tick: u64       (8 bytes)
/// - frame_end_tick:   u64       (8 bytes)
/// - group_count:      u16       (2 bytes)
/// - _reserved:        [u8; 6]   (6 bytes)
///
/// Followed by `group_count` entries of 10 bytes each:
/// - group_id: u16 (2)
/// - offset:   u32 (4)
/// - byte_len: u32 (4)
#[derive(Debug, Clone, PartialEq)]
pub struct RowGroupHeader {
    pub row_count: u32,
    pub frame_start_tick: u64,
    pub frame_end_tick: u64,
    pub groups: Vec<GroupEntryV2>,
}

impl RowGroupHeader {
    pub const FIXED_SIZE: usize = 32;
    pub const ENTRY_SIZE: usize = 10;

    pub fn byte_len(&self) -> usize {
        Self::FIXED_SIZE + self.groups.len() * Self::ENTRY_SIZE
    }

    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&MAGIC_RGHD)?;
        writer.write_all(&self.row_count.to_le_bytes())?;
        writer.write_all(&self.frame_start_tick.to_le_bytes())?;
        writer.write_all(&self.frame_end_tick.to_le_bytes())?;
        writer.write_all(&(self.groups.len() as u16).to_le_bytes())?;
        writer.write_all(&[0u8; 6])?;
        for entry in &self.groups {
            writer.write_all(&entry.group_id.to_le_bytes())?;
            writer.write_all(&entry.offset.to_le_bytes())?;
            writer.write_all(&entry.byte_len.to_le_bytes())?;
        }
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != MAGIC_RGHD {
            return Err(TelemetryError::InvalidFormat(
                "bad row group header magic".to_string(),
            ));
        }
        let row_count = read_u32(reader)?;
        let frame_start_tick = read_u64(reader)?;
        let frame_end_tick = read_u64(reader)?;
        let group_count = read_u16(reader)?;
        let mut reserved = [0u8; 6];
        reader.read_exact(&mut reserved)?;
        let mut groups = Vec::with_capacity(group_count as usize);
        for _ in 0..group_count {
            let group_id = read_u16(reader)?;
            let offset = read_u32(reader)?;
            let byte_len = read_u32(reader)?;
            groups.push(GroupEntryV2 {
                group_id,
                offset,
                byte_len,
            });
        }
        Ok(Self {
            row_count,
            frame_start_tick,
            frame_end_tick,
            groups,
        })
    }
}

// ---------------------------------------------------------------------------
// ColumnEntryV2 — column statistics / offset within a group block
// ---------------------------------------------------------------------------

/// V2 column entry (40 bytes).
///
/// Layout:
/// - column_id:  u16   (2 bytes)
/// - codec:      u8    (1 byte)
/// - value_type: u8    (1 byte)
/// - byte_len:   u32   (4 bytes)
/// - crc32:      u32   (4 bytes)
/// - min_value:  f64   (8 bytes)
/// - max_value:  f64   (8 bytes)
/// - _reserved:  [u8; 12] (12 bytes padding)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnEntryV2 {
    pub column_id: u16,
    pub codec: u8,
    pub value_type: u8,
    pub byte_len: u32,
    pub crc32: u32,
    pub min_value: f64,
    pub max_value: f64,
}

impl ColumnEntryV2 {
    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&self.column_id.to_le_bytes())?;
        writer.write_all(&[self.codec, self.value_type])?;
        writer.write_all(&self.byte_len.to_le_bytes())?;
        writer.write_all(&self.crc32.to_le_bytes())?;
        writer.write_all(&self.min_value.to_le_bytes())?;
        writer.write_all(&self.max_value.to_le_bytes())?;
        writer.write_all(&[0u8; 12])?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let column_id = read_u16(reader)?;
        let mut meta = [0u8; 2];
        reader.read_exact(&mut meta)?;
        let codec = meta[0];
        let value_type = meta[1];
        let byte_len = read_u32(reader)?;
        let crc32 = read_u32(reader)?;
        let min_value = read_f64(reader)?;
        let max_value = read_f64(reader)?;
        let mut reserved = [0u8; 12];
        reader.read_exact(&mut reserved)?;
        Ok(Self {
            column_id,
            codec,
            value_type,
            byte_len,
            crc32,
            min_value,
            max_value,
        })
    }
}

// ---------------------------------------------------------------------------
// SkipIndexEntry — time-range → row-group lookup
// ---------------------------------------------------------------------------

/// A single skip-index entry mapping a time-range (column × frame window)
/// to a row group and byte-offset within it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkipIndexEntry {
    pub access_group: u16,
    pub column_id: u16,
    pub frame_start: u64,
    pub frame_end: u64,
    pub row_group_index: u32,
    pub offset_in_group: u32,
    pub byte_len: u32,
}

impl SkipIndexEntry {
    pub const BYTE_LEN: usize = 32;

    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&self.access_group.to_le_bytes())?;
        writer.write_all(&self.column_id.to_le_bytes())?;
        writer.write_all(&self.frame_start.to_le_bytes())?;
        writer.write_all(&self.frame_end.to_le_bytes())?;
        writer.write_all(&self.row_group_index.to_le_bytes())?;
        writer.write_all(&self.offset_in_group.to_le_bytes())?;
        writer.write_all(&self.byte_len.to_le_bytes())?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let access_group = read_u16(reader)?;
        let column_id = read_u16(reader)?;
        let frame_start = read_u64(reader)?;
        let frame_end = read_u64(reader)?;
        let row_group_index = read_u32(reader)?;
        let offset_in_group = read_u32(reader)?;
        let byte_len = read_u32(reader)?;
        Ok(Self {
            access_group,
            column_id,
            frame_start,
            frame_end,
            row_group_index,
            offset_in_group,
            byte_len,
        })
    }
}

// ---------------------------------------------------------------------------
// LapIndexEntryV2 — lap boundaries
// ---------------------------------------------------------------------------

/// V2 lap index entry (32 bytes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LapIndexEntryV2 {
    pub lap_number: i32,
    pub start_tick: u64,
    pub end_tick: u64,
    pub sample_count: u32,
    pub is_valid: i32,
    pub is_out_lap: i32,
}

impl LapIndexEntryV2 {
    pub const BYTE_LEN: usize = 32;

    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&self.lap_number.to_le_bytes())?;
        writer.write_all(&self.start_tick.to_le_bytes())?;
        writer.write_all(&self.end_tick.to_le_bytes())?;
        writer.write_all(&self.sample_count.to_le_bytes())?;
        writer.write_all(&self.is_valid.to_le_bytes())?;
        writer.write_all(&self.is_out_lap.to_le_bytes())?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        let lap_number = i32::from_le_bytes(buf);
        let start_tick = read_u64(reader)?;
        let end_tick = read_u64(reader)?;
        let sample_count = read_u32(reader)?;
        reader.read_exact(&mut buf)?;
        let is_valid = i32::from_le_bytes(buf);
        reader.read_exact(&mut buf)?;
        let is_out_lap = i32::from_le_bytes(buf);
        Ok(Self {
            lap_number,
            start_tick,
            end_tick,
            sample_count,
            is_valid,
            is_out_lap,
        })
    }
}

// ---------------------------------------------------------------------------
// FooterV2 — file footer with index entries
// ---------------------------------------------------------------------------

/// V2 file footer.
///
/// Fixed portion:
/// - footer_magic:     b"FTR2" (4 bytes)
/// - footer_offset:    u64     (8 bytes)
/// - skip_index_count: u32     (4 bytes)
/// - lap_index_count:  u32     (4 bytes)
///
/// Followed by `skip_index_count` SkipIndexEntry entries,
/// then `lap_index_count` LapIndexEntryV2 entries.
#[derive(Debug, Clone, PartialEq)]
pub struct FooterV2 {
    pub footer_offset: u64,
    pub skip_index_count: u32,
    pub lap_index_count: u32,
}

impl FooterV2 {
    pub const FIXED_SIZE: usize = 20;

    pub fn write_to(&self, writer: &mut impl Write) -> TelemetryResult<()> {
        writer.write_all(&MAGIC_FTR2)?;
        writer.write_all(&self.footer_offset.to_le_bytes())?;
        writer.write_all(&self.skip_index_count.to_le_bytes())?;
        writer.write_all(&self.lap_index_count.to_le_bytes())?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> TelemetryResult<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != MAGIC_FTR2 {
            return Err(TelemetryError::InvalidFormat(
                "bad footer magic".to_string(),
            ));
        }
        let footer_offset = read_u64(reader)?;
        let skip_index_count = read_u32(reader)?;
        let lap_index_count = read_u32(reader)?;
        Ok(Self {
            footer_offset,
            skip_index_count,
            lap_index_count,
        })
    }

    pub fn read_skip_entries(
        &self,
        reader: &mut impl Read,
    ) -> TelemetryResult<Vec<SkipIndexEntry>> {
        let mut entries = Vec::with_capacity(self.skip_index_count as usize);
        for _ in 0..self.skip_index_count {
            entries.push(SkipIndexEntry::read_from(reader)?);
        }
        Ok(entries)
    }

    pub fn write_skip_entries(
        &self,
        writer: &mut impl Write,
        entries: &[SkipIndexEntry],
    ) -> TelemetryResult<()> {
        if entries.len() as u32 != self.skip_index_count {
            return Err(TelemetryError::InvalidArgument(format!(
                "skip index count mismatch: expected {} got {}",
                self.skip_index_count,
                entries.len(),
            )));
        }
        for entry in entries {
            entry.write_to(writer)?;
        }
        Ok(())
    }

    pub fn read_lap_entries(
        &self,
        reader: &mut impl Read,
    ) -> TelemetryResult<Vec<LapIndexEntryV2>> {
        let mut entries = Vec::with_capacity(self.lap_index_count as usize);
        for _ in 0..self.lap_index_count {
            entries.push(LapIndexEntryV2::read_from(reader)?);
        }
        Ok(entries)
    }

    pub fn write_lap_entries(
        &self,
        writer: &mut impl Write,
        entries: &[LapIndexEntryV2],
    ) -> TelemetryResult<()> {
        if entries.len() as u32 != self.lap_index_count {
            return Err(TelemetryError::InvalidArgument(format!(
                "lap index count mismatch: expected {} got {}",
                self.lap_index_count,
                entries.len(),
            )));
        }
        for entry in entries {
            entry.write_to(writer)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// V2 Format Type Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests_v2 {
    use super::*;

    #[test]
    fn test_header_v2_roundtrip() {
        let header = FileHeaderV2 {
            schema_offset: 100,
            metadata_offset: 200,
            first_row_group_offset: 300,
            footer_offset: 400,
            created_unix_ns: 1_234_567_890_000,
            poll_hz: 120_000,
        };
        let mut buf = Vec::new();
        header.write_to(&mut buf).unwrap();
        assert_eq!(buf.len(), HEADER_V2_SIZE);

        let mut cursor = std::io::Cursor::new(&buf);
        let read_back = FileHeaderV2::read_from(&mut cursor).unwrap();
        assert_eq!(header, read_back);
    }

    #[test]
    fn test_header_v2_bad_magic() {
        let bad = [0u8; 64];
        let mut cursor = std::io::Cursor::new(&bad);
        let result = FileHeaderV2::read_from(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_header_v2_bad_version() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"ACT2");
        buf.extend_from_slice(&99u16.to_le_bytes()); // bad version
        buf.resize(HEADER_V2_SIZE, 0);
        let mut cursor = std::io::Cursor::new(&buf);
        let result = FileHeaderV2::read_from(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_schema_v2_roundtrip() {
        let schema = SchemaBlockV2 {
            groups: vec![
                SchemaGroupDefV2 {
                    group_id: 1,
                    columns: vec![
                        SchemaColumnDefV2 {
                            column_id: 10,
                            value_type: TYPE_F32,
                            name: "speed_kmh".to_string(),
                        },
                        SchemaColumnDefV2 {
                            column_id: 11,
                            value_type: TYPE_F32,
                            name: "gas".to_string(),
                        },
                    ],
                },
                SchemaGroupDefV2 {
                    group_id: 2,
                    columns: vec![SchemaColumnDefV2 {
                        column_id: 20,
                        value_type: TYPE_F64,
                        name: "velocity".to_string(),
                    }],
                },
            ],
        };
        let mut buf = Vec::new();
        schema.write_to(&mut buf).unwrap();

        let mut cursor = std::io::Cursor::new(&buf);
        let read_back = SchemaBlockV2::read_from(&mut cursor).unwrap();
        assert_eq!(schema, read_back);
    }

    #[test]
    fn test_column_entry_v2_roundtrip() {
        let entry = ColumnEntryV2 {
            column_id: 10,
            codec: 0,
            value_type: TYPE_F32,
            byte_len: 4096,
            crc32: 0xDEAD_BEEF,
            min_value: 0.0,
            max_value: 320.5,
        };
        let mut buf = Vec::new();
        entry.write_to(&mut buf).unwrap();
        assert_eq!(buf.len(), COLUMN_ENTRY_V2_SIZE);

        let mut cursor = std::io::Cursor::new(&buf);
        let read_back = ColumnEntryV2::read_from(&mut cursor).unwrap();
        assert_eq!(entry, read_back);
    }

    #[test]
    fn test_skip_index_entry_roundtrip() {
        let entry = SkipIndexEntry {
            access_group: 1,
            column_id: 10,
            frame_start: 0,
            frame_end: 1000,
            row_group_index: 0,
            offset_in_group: 128,
            byte_len: 4096,
        };
        let mut buf = Vec::new();
        entry.write_to(&mut buf).unwrap();
        assert_eq!(buf.len(), SkipIndexEntry::BYTE_LEN);

        let mut cursor = std::io::Cursor::new(&buf);
        let read_back = SkipIndexEntry::read_from(&mut cursor).unwrap();
        assert_eq!(entry, read_back);
    }

    #[test]
    fn test_lap_index_entry_v2_roundtrip() {
        let entry = LapIndexEntryV2 {
            lap_number: 3,
            start_tick: 1000,
            end_tick: 5000,
            sample_count: 400,
            is_valid: 1,
            is_out_lap: 0,
        };
        let mut buf = Vec::new();
        entry.write_to(&mut buf).unwrap();
        assert_eq!(buf.len(), LapIndexEntryV2::BYTE_LEN);

        let mut cursor = std::io::Cursor::new(&buf);
        let read_back = LapIndexEntryV2::read_from(&mut cursor).unwrap();
        assert_eq!(entry, read_back);
    }

    #[test]
    fn test_row_group_header_roundtrip() {
        let header = RowGroupHeader {
            row_count: 1024,
            frame_start_tick: 0,
            frame_end_tick: 1023,
            groups: vec![
                GroupEntryV2 {
                    group_id: 1,
                    offset: 64,
                    byte_len: 4096,
                },
                GroupEntryV2 {
                    group_id: 2,
                    offset: 4160,
                    byte_len: 2048,
                },
            ],
        };
        let mut buf = Vec::new();
        header.write_to(&mut buf).unwrap();
        assert_eq!(buf.len(), header.byte_len());

        let mut cursor = std::io::Cursor::new(&buf);
        let read_back = RowGroupHeader::read_from(&mut cursor).unwrap();
        assert_eq!(header, read_back);
    }

    #[test]
    fn test_footer_v2_skip_entries() {
        let skip_entries = vec![
            SkipIndexEntry {
                access_group: 1,
                column_id: 10,
                frame_start: 0,
                frame_end: 500,
                row_group_index: 0,
                offset_in_group: 0,
                byte_len: 100,
            },
            SkipIndexEntry {
                access_group: 1,
                column_id: 10,
                frame_start: 501,
                frame_end: 1000,
                row_group_index: 1,
                offset_in_group: 0,
                byte_len: 100,
            },
        ];
        let footer = FooterV2 {
            footer_offset: 9999,
            skip_index_count: skip_entries.len() as u32,
            lap_index_count: 0,
        };

        let mut buf = Vec::new();
        footer.write_to(&mut buf).unwrap();
        footer.write_skip_entries(&mut buf, &skip_entries).unwrap();

        let mut cursor = std::io::Cursor::new(&buf);
        let read_footer = FooterV2::read_from(&mut cursor).unwrap();
        assert_eq!(footer, read_footer);

        let read_skips = read_footer.read_skip_entries(&mut cursor).unwrap();
        assert_eq!(skip_entries, read_skips);
    }

    #[test]
    fn test_footer_v2_lap_entries() {
        let lap_entries = vec![
            LapIndexEntryV2 {
                lap_number: 1,
                start_tick: 0,
                end_tick: 5000,
                sample_count: 500,
                is_valid: 1,
                is_out_lap: 0,
            },
            LapIndexEntryV2 {
                lap_number: 2,
                start_tick: 5001,
                end_tick: 10000,
                sample_count: 499,
                is_valid: 1,
                is_out_lap: 0,
            },
        ];
        let footer = FooterV2 {
            footer_offset: 9999,
            skip_index_count: 0,
            lap_index_count: lap_entries.len() as u32,
        };

        let mut buf = Vec::new();
        footer.write_to(&mut buf).unwrap();
        footer.write_lap_entries(&mut buf, &lap_entries).unwrap();

        let mut cursor = std::io::Cursor::new(&buf);
        let read_footer = FooterV2::read_from(&mut cursor).unwrap();
        assert_eq!(footer, read_footer);

        let read_laps = read_footer.read_lap_entries(&mut cursor).unwrap();
        assert_eq!(lap_entries, read_laps);
    }
}
