//! V2 binary telemetry writer — row-group-based columnar encoding.
use crate::encode_v2::{encode_column, CODEC_PLAIN};
use crate::error::TelemetryResult;
use crate::format_v2::{
    ColumnEntryV2, ColumnId, FileHeaderV2, FooterV2, GroupEntryV2, GroupId, LapIndexEntryV2,
    RowGroupHeader, SchemaBlockV2, SchemaColumnDefV2, SchemaGroupDefV2, SkipIndexEntry,
    COLUMN_ENTRY_V2_SIZE, HEADER_V2_SIZE, TYPE_BYTES, TYPE_BYTES_F32, TYPE_BYTES_I32,
    TYPE_BYTES_U16, TYPE_F32, TYPE_I32, TYPE_U64,
};
use crate::types::{RecordingSummary, SessionMetadata};
use crate::writer::{encode_metadata, LiveTelemetryConfig, TelemetryFrame};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

fn arr4(arr: [f32; 4]) -> (Vec<f64>, u8) {
    (
        vec![arr[0] as f64, arr[1] as f64, arr[2] as f64, arr[3] as f64],
        TYPE_BYTES_F32,
    )
}
fn arr12(arr: [f32; 12]) -> (Vec<f64>, u8) {
    (arr.iter().map(|&v| v as f64).collect(), TYPE_BYTES_F32)
}
fn arr33(arr: [u16; 33]) -> (Vec<f64>, u8) {
    (arr.iter().map(|&v| v as f64).collect(), TYPE_BYTES_U16)
}
fn arr15(arr: [u16; 15]) -> (Vec<f64>, u8) {
    (arr.iter().map(|&v| v as f64).collect(), TYPE_BYTES_U16)
}
fn arr5(arr: [f32; 5]) -> (Vec<f64>, u8) {
    (arr.iter().map(|&v| v as f64).collect(), TYPE_BYTES_F32)
}
fn arr2(arr: [f32; 2]) -> (Vec<f64>, u8) {
    (vec![arr[0] as f64, arr[1] as f64], TYPE_BYTES_F32)
}

fn extract_column_values(frame: &TelemetryFrame, cid: u16) -> (Vec<f64>, u8) {
    match cid {
        1 => (vec![frame.sample_tick as f64], TYPE_U64),
        2 => (vec![frame.timestamp_ns as f64], TYPE_U64),
        3 => (vec![frame.controls.physics_packet_id as f64], TYPE_I32),
        4 => (vec![frame.controls.graphics_packet_id as f64], TYPE_I32),
        10 => (vec![frame.controls.speed_kmh as f64], TYPE_F32),
        11 => (vec![frame.controls.gas as f64], TYPE_F32),
        12 => (vec![frame.controls.brake as f64], TYPE_F32),
        13 => (vec![frame.controls.clutch as f64], TYPE_F32),
        14 => (vec![frame.controls.steer_angle as f64], TYPE_F32),
        15 => (vec![frame.controls.gear as f64], TYPE_I32),
        16 => (vec![frame.controls.rpms as f64], TYPE_I32),
        17 => (vec![frame.controls.fuel as f64], TYPE_F32),
        60 => (vec![frame.powertrain.turbo_boost as f64], TYPE_F32),
        61 => (vec![frame.powertrain.ballast as f64], TYPE_F32),
        62 => (vec![frame.powertrain.kers_charge as f64], TYPE_F32),
        63 => (vec![frame.powertrain.kers_input as f64], TYPE_F32),
        64 => (vec![frame.powertrain.kers_current_kj as f64], TYPE_F32),
        65 => (vec![frame.powertrain.drs as f64], TYPE_F32),
        66 => (vec![frame.powertrain.tc as f64], TYPE_F32),
        67 => (vec![frame.powertrain.abs as f64], TYPE_F32),
        68 => (vec![frame.powertrain.engine_brake as f64], TYPE_I32),
        69 => (vec![frame.powertrain.ers_recovery_level as f64], TYPE_I32),
        70 => (vec![frame.powertrain.ers_power_level as f64], TYPE_I32),
        71 => (vec![frame.powertrain.ers_heat_charging as f64], TYPE_I32),
        72 => (vec![frame.powertrain.ers_is_charging as f64], TYPE_I32),
        73 => (vec![frame.powertrain.drs_available as f64], TYPE_I32),
        74 => (vec![frame.powertrain.drs_enabled as f64], TYPE_I32),
        75 => (vec![frame.powertrain.tc_in_action as f64], TYPE_I32),
        76 => (vec![frame.powertrain.abs_in_action as f64], TYPE_I32),
        77 => (vec![frame.powertrain.auto_shifter_on as f64], TYPE_I32),
        78 => (vec![frame.powertrain.current_max_rpm as f64], TYPE_I32),
        79 => (vec![frame.powertrain.p2p_activations as f64], TYPE_I32),
        80 => (vec![frame.powertrain.p2p_status as f64], TYPE_I32),
        81 => (vec![frame.powertrain.water_temp as f64], TYPE_F32),
        20 => {
            let v = frame.motion.velocity;
            (vec![v[0] as f64, v[1] as f64, v[2] as f64], TYPE_BYTES_F32)
        }
        21 => {
            let a = frame.motion.acc_g;
            (vec![a[0] as f64, a[1] as f64, a[2] as f64], TYPE_BYTES_F32)
        }
        22 => {
            let lv = frame.motion.local_velocity;
            (
                vec![lv[0] as f64, lv[1] as f64, lv[2] as f64],
                TYPE_BYTES_F32,
            )
        }
        23 => {
            let lav = frame.motion.local_angular_vel;
            (
                vec![lav[0] as f64, lav[1] as f64, lav[2] as f64],
                TYPE_BYTES_F32,
            )
        }
        24 => (vec![frame.motion.heading as f64], TYPE_F32),
        25 => (vec![frame.motion.pitch as f64], TYPE_F32),
        26 => (vec![frame.motion.roll as f64], TYPE_F32),
        30 => arr4(frame.tyres.wheel_slip),
        31 => arr4(frame.tyres.wheel_load),
        32 => arr4(frame.tyres.wheels_pressure),
        33 => arr4(frame.tyres.wheel_angular_speed),
        34 => arr4(frame.tyres.tyre_wear),
        35 => arr4(frame.tyres.tyre_dirty_level),
        36 => arr4(frame.tyres.tyre_core_temperature),
        37 => arr4(frame.tyres.camber_rad),
        38 => arr4(frame.tyres.suspension_travel),
        39 => arr4(frame.tyres.slip_ratio),
        40 => arr4(frame.tyres.slip_angle),
        41 => arr4(frame.tyres.tyre_temp_i),
        42 => arr4(frame.tyres.tyre_temp_m),
        43 => arr4(frame.tyres.tyre_temp_o),
        44 => arr4(frame.tyres.tyre_temp),
        45 => arr4(frame.tyres.mz),
        46 => arr4(frame.tyres.fx),
        47 => arr4(frame.tyres.fy),
        48 => arr4(frame.tyres.suspension_damage),
        49 => arr4(frame.tyres.brake_temp),
        50 => arr4(frame.tyres.brake_pressure),
        51 => arr4(frame.tyres.pad_life),
        52 => arr4(frame.tyres.disc_life),
        53 => arr12(frame.tyres.tyre_contact_point),
        54 => arr12(frame.tyres.tyre_contact_normal),
        55 => arr12(frame.tyres.tyre_contact_heading),
        56 => (vec![frame.tyres.number_of_tyres_out as f64], TYPE_I32),
        57 => (vec![frame.tyres.front_brake_compound as f64], TYPE_I32),
        58 => (vec![frame.tyres.rear_brake_compound as f64], TYPE_I32),
        90 => (vec![frame.session.status as f64], TYPE_I32),
        91 => (vec![frame.session.session as f64], TYPE_I32),
        92 => (vec![frame.session.session_index as f64], TYPE_I32),
        93 => (vec![frame.session.completed_laps as f64], TYPE_I32),
        94 => (vec![frame.session.position as f64], TYPE_I32),
        95 => (vec![frame.session.session_time_left as f64], TYPE_F32),
        96 => (vec![frame.session.number_of_laps as f64], TYPE_I32),
        97 => (vec![frame.session.current_sector_index as f64], TYPE_I32),
        98 => (vec![frame.session.normalized_car_position as f64], TYPE_F32),
        99 => (vec![frame.session.is_in_pit as f64], TYPE_I32),
        100 => (vec![frame.session.is_in_pit_lane as f64], TYPE_I32),
        101 => (vec![frame.session.mandatory_pit_done as f64], TYPE_I32),
        102 => (vec![frame.session.missing_mandatory_pits as f64], TYPE_I32),
        103 => (vec![frame.session.penalty_time as f64], TYPE_F32),
        104 => (vec![frame.session.penalty_type as f64], TYPE_I32),
        105 => arr33(frame.session.track_status),
        106 => (vec![frame.session.clock as f64], TYPE_F32),
        107 => (vec![frame.session.replay_time_multiplier as f64], TYPE_F32),
        108 => (vec![frame.session.is_valid_lap as f64], TYPE_I32),
        109 => (vec![frame.session.global_yellow as f64], TYPE_I32),
        110 => (vec![frame.session.global_yellow1 as f64], TYPE_I32),
        111 => (vec![frame.session.global_yellow2 as f64], TYPE_I32),
        112 => (vec![frame.session.global_yellow3 as f64], TYPE_I32),
        113 => (vec![frame.session.global_white as f64], TYPE_I32),
        114 => (vec![frame.session.global_green as f64], TYPE_I32),
        115 => (vec![frame.session.global_chequered as f64], TYPE_I32),
        116 => (vec![frame.session.global_red as f64], TYPE_I32),
        117 => (vec![frame.session.gap_ahead_or_tail_value as f64], TYPE_I32),
        118 => (vec![frame.session.flag as f64], TYPE_I32),
        119 => (vec![frame.session.gap_behind as f64], TYPE_I32),
        120 => (vec![frame.timing.i_current_time as f64], TYPE_I32),
        121 => (vec![frame.timing.i_last_time as f64], TYPE_I32),
        122 => (vec![frame.timing.i_best_time as f64], TYPE_I32),
        123 => (vec![frame.timing.i_split as f64], TYPE_I32),
        124 => (vec![frame.timing.last_sector_time as f64], TYPE_I32),
        125 => (vec![frame.timing.i_delta_lap_time as f64], TYPE_I32),
        126 => (vec![frame.timing.is_delta_positive as f64], TYPE_I32),
        127 => (vec![frame.timing.i_estimated_lap_time as f64], TYPE_I32),
        128 => (vec![frame.timing.fuel_estimated_laps as f64], TYPE_F32),
        129 => (vec![frame.timing.fuel_x_lap as f64], TYPE_F32),
        130 => (vec![frame.timing.used_fuel as f64], TYPE_F32),
        131 => (vec![frame.timing.distance_traveled as f64], TYPE_F32),
        132 => arr15(frame.timing.current_time_str),
        133 => arr15(frame.timing.last_time_str),
        134 => arr15(frame.timing.best_time_str),
        135 => arr15(frame.timing.split_str),
        136 => arr15(frame.timing.delta_lap_time_str),
        137 => arr15(frame.timing.estimated_lap_time_str),
        138 => (
            vec![frame.timing.observed_slot_before_i_split as f64],
            TYPE_I32,
        ),
        150 => arr5(frame.car_state.car_damage),
        151 => (vec![frame.car_state.pit_limiter_on as f64], TYPE_I32),
        152 => arr2(frame.car_state.ride_height),
        153 => (vec![frame.car_state.ignition_on as f64], TYPE_I32),
        154 => (vec![frame.car_state.starter_engine_on as f64], TYPE_I32),
        155 => (vec![frame.car_state.is_engine_running as f64], TYPE_I32),
        156 => (vec![frame.car_state.is_ai_controlled as f64], TYPE_I32),
        157 => (vec![frame.car_state.cg_height as f64], TYPE_F32),
        158 => (vec![frame.car_state.brake_bias as f64], TYPE_F32),
        159 => (vec![frame.car_state.rain_lights as f64], TYPE_I32),
        160 => (vec![frame.car_state.flashing_lights as f64], TYPE_I32),
        161 => (vec![frame.car_state.lights_stage as f64], TYPE_I32),
        162 => (vec![frame.car_state.wiper_lv as f64], TYPE_I32),
        163 => (
            vec![frame.car_state.driver_stint_total_time_left as f64],
            TYPE_I32,
        ),
        164 => (
            vec![frame.car_state.driver_stint_time_left as f64],
            TYPE_I32,
        ),
        165 => (vec![frame.car_state.rain_tyres as f64], TYPE_I32),
        166 => (vec![frame.car_state.current_tyre_set as f64], TYPE_I32),
        167 => (vec![frame.car_state.strategy_tyre_set as f64], TYPE_I32),
        168 => (vec![frame.car_state.track_grip_status as f64], TYPE_I32),
        169 => arr33(frame.car_state.tyre_compound_str),
        170 => (vec![frame.car_state.mfd_tyre_set as f64], TYPE_I32),
        171 => (vec![frame.car_state.mfd_fuel_to_add as f64], TYPE_F32),
        172 => arr4(frame.car_state.mfd_tyre_pressure),
        173 => (vec![frame.car_state.ideal_line_on as f64], TYPE_I32),
        174 => (vec![frame.car_state.is_setup_menu_visible as f64], TYPE_I32),
        175 => (vec![frame.car_state.main_display_index as f64], TYPE_I32),
        176 => (
            vec![frame.car_state.secondary_display_index as f64],
            TYPE_I32,
        ),
        177 => (vec![frame.car_state.direction_lights_left as f64], TYPE_I32),
        178 => (
            vec![frame.car_state.direction_lights_right as f64],
            TYPE_I32,
        ),
        179 => (vec![frame.car_state.tc_level as f64], TYPE_I32),
        180 => (vec![frame.car_state.tc_cut as f64], TYPE_I32),
        181 => (vec![frame.car_state.engine_map as f64], TYPE_I32),
        182 => (vec![frame.car_state.abs_level as f64], TYPE_I32),
        183 => (vec![frame.car_state.exhaust_temperature as f64], TYPE_F32),
        184 => (vec![frame.car_state.final_ff as f64], TYPE_F32),
        185 => (vec![frame.car_state.performance_meter as f64], TYPE_F32),
        186 => (vec![frame.car_state.kerb_vibration as f64], TYPE_F32),
        187 => (vec![frame.car_state.slip_vibrations as f64], TYPE_F32),
        188 => (vec![frame.car_state.g_vibrations as f64], TYPE_F32),
        189 => (vec![frame.car_state.abs_vibrations as f64], TYPE_F32),
        200 => (vec![frame.environment.air_density as f64], TYPE_F32),
        201 => (vec![frame.environment.air_temp as f64], TYPE_F32),
        202 => (vec![frame.environment.road_temp as f64], TYPE_F32),
        203 => (vec![frame.environment.wind_speed as f64], TYPE_F32),
        204 => (vec![frame.environment.wind_direction as f64], TYPE_F32),
        205 => (vec![frame.environment.surface_grip as f64], TYPE_F32),
        206 => (vec![frame.environment.rain_intensity as f64], TYPE_I32),
        207 => (
            vec![frame.environment.rain_intensity_in_10min as f64],
            TYPE_I32,
        ),
        208 => (
            vec![frame.environment.rain_intensity_in_30min as f64],
            TYPE_I32,
        ),
        210 => (vec![frame.other_cars.active_cars as f64], TYPE_I32),
        211 => (vec![frame.other_cars.player_car_id as f64], TYPE_I32),
        212 => (
            frame
                .other_cars
                .car_coordinates
                .iter()
                .map(|&c| c as f64)
                .collect(),
            TYPE_BYTES_F32,
        ),
        213 => (
            frame
                .other_cars
                .car_id
                .iter()
                .map(|&id| id as f64)
                .collect(),
            TYPE_BYTES_I32,
        ),
        _ => (vec![], 0),
    }
}

fn build_default_schema() -> SchemaBlockV2 {
    let all_ids: &[(u16, &str)] = &[
        (1, "sample_tick"),
        (2, "timestamp_ns"),
        (3, "physics_packet_id"),
        (4, "graphics_packet_id"),
        (10, "speed_kmh"),
        (11, "gas"),
        (12, "brake"),
        (13, "clutch"),
        (14, "steer_angle"),
        (15, "gear"),
        (16, "rpms"),
        (17, "fuel"),
        (20, "velocity"),
        (21, "acc_g"),
        (22, "local_velocity"),
        (23, "local_angular_vel"),
        (24, "heading"),
        (25, "pitch"),
        (26, "roll"),
        (30, "wheel_slip"),
        (31, "wheel_load"),
        (32, "wheels_pressure"),
        (33, "wheel_angular_speed"),
        (34, "tyre_wear"),
        (35, "tyre_dirty_level"),
        (36, "tyre_core_temperature"),
        (37, "camber_rad"),
        (38, "suspension_travel"),
        (39, "slip_ratio"),
        (40, "slip_angle"),
        (41, "tyre_temp_i"),
        (42, "tyre_temp_m"),
        (43, "tyre_temp_o"),
        (44, "tyre_temp"),
        (45, "mz"),
        (46, "fx"),
        (47, "fy"),
        (48, "suspension_damage"),
        (49, "brake_temp"),
        (50, "brake_pressure"),
        (51, "pad_life"),
        (52, "disc_life"),
        (53, "tyre_contact_point"),
        (54, "tyre_contact_normal"),
        (55, "tyre_contact_heading"),
        (56, "number_of_tyres_out"),
        (57, "front_brake_compound"),
        (58, "rear_brake_compound"),
        (60, "turbo_boost"),
        (61, "ballast"),
        (62, "kers_charge"),
        (63, "kers_input"),
        (64, "kers_current_kj"),
        (65, "drs"),
        (66, "tc_physics"),
        (67, "abs_physics"),
        (68, "engine_brake"),
        (69, "ers_recovery_level"),
        (70, "ers_power_level"),
        (71, "ers_heat_charging"),
        (72, "ers_is_charging"),
        (73, "drs_available"),
        (74, "drs_enabled"),
        (75, "tc_in_action"),
        (76, "abs_in_action"),
        (77, "auto_shifter_on"),
        (78, "current_max_rpm"),
        (79, "p2p_activations"),
        (80, "p2p_status"),
        (81, "water_temp"),
        (90, "status"),
        (91, "session"),
        (92, "session_index"),
        (93, "completed_laps"),
        (94, "position"),
        (95, "session_time_left"),
        (96, "number_of_laps"),
        (97, "current_sector_index"),
        (98, "normalized_car_position"),
        (99, "is_in_pit"),
        (100, "is_in_pit_lane"),
        (101, "mandatory_pit_done"),
        (102, "missing_mandatory_pits"),
        (103, "penalty_time"),
        (104, "penalty_type"),
        (105, "track_status"),
        (106, "clock"),
        (107, "replay_time_multiplier"),
        (108, "is_valid_lap"),
        (109, "global_yellow"),
        (110, "global_yellow1"),
        (111, "global_yellow2"),
        (112, "global_yellow3"),
        (113, "global_white"),
        (114, "global_green"),
        (115, "global_chequered"),
        (116, "global_red"),
        (117, "gap_ahead_or_tail_value"),
        (118, "flag"),
        (119, "gap_behind"),
        (120, "i_current_time"),
        (121, "i_last_time"),
        (122, "i_best_time"),
        (123, "i_split"),
        (124, "last_sector_time"),
        (125, "i_delta_lap_time"),
        (126, "is_delta_positive"),
        (127, "i_estimated_lap_time"),
        (128, "fuel_estimated_laps"),
        (129, "fuel_x_lap"),
        (130, "used_fuel"),
        (131, "distance_traveled"),
        (132, "current_time_str"),
        (133, "last_time_str"),
        (134, "best_time_str"),
        (135, "split_str"),
        (136, "delta_lap_time_str"),
        (137, "estimated_lap_time_str"),
        (138, "observed_slot_before_i_split"),
        (150, "car_damage"),
        (151, "pit_limiter_on"),
        (152, "ride_height"),
        (153, "ignition_on"),
        (154, "starter_engine_on"),
        (155, "is_engine_running"),
        (156, "is_ai_controlled"),
        (157, "cg_height"),
        (158, "brake_bias"),
        (159, "rain_lights"),
        (160, "flashing_lights"),
        (161, "lights_stage"),
        (162, "wiper_lv"),
        (163, "driver_stint_total_time_left"),
        (164, "driver_stint_time_left"),
        (165, "rain_tyres"),
        (166, "current_tyre_set"),
        (167, "strategy_tyre_set"),
        (168, "track_grip_status"),
        (169, "tyre_compound_str"),
        (170, "mfd_tyre_set"),
        (171, "mfd_fuel_to_add"),
        (172, "mfd_tyre_pressure"),
        (173, "ideal_line_on"),
        (174, "is_setup_menu_visible"),
        (175, "main_display_index"),
        (176, "secondary_display_index"),
        (177, "direction_lights_left"),
        (178, "direction_lights_right"),
        (179, "tc_level"),
        (180, "tc_cut"),
        (181, "engine_map"),
        (182, "abs_level"),
        (183, "exhaust_temperature"),
        (184, "final_ff"),
        (185, "performance_meter"),
        (186, "kerb_vibration"),
        (187, "slip_vibrations"),
        (188, "g_vibrations"),
        (189, "abs_vibrations"),
        (200, "air_density"),
        (201, "air_temp"),
        (202, "road_temp"),
        (203, "wind_speed"),
        (204, "wind_direction"),
        (205, "surface_grip"),
        (206, "rain_intensity"),
        (207, "rain_intensity_in_10min"),
        (208, "rain_intensity_in_30min"),
        (210, "active_cars"),
        (211, "player_car_id"),
        (212, "car_coordinates"),
        (213, "car_id"),
    ];
    let group_order: &[(u16, &str)] = &[
        (0, "frame_meta"),
        (1, "driver_inputs"),
        (2, "vehicle_dynamics"),
        (3, "tyres"),
        (4, "timing"),
        (5, "environment"),
        (6, "cold_storage"),
    ];
    let mut groups = Vec::new();
    for &(gid, _gname) in group_order {
        let mut columns = Vec::new();
        for &(cid, name) in all_ids {
            let group = crate::format_v2::column_group(unsafe {
                std::mem::transmute::<u16, ColumnId>(cid)
            });
            if (group as u16) == gid {
                columns.push(SchemaColumnDefV2 {
                    column_id: cid,
                    value_type: 0,
                    name: name.to_string(),
                });
            }
        }
        groups.push(SchemaGroupDefV2 {
            group_id: gid,
            columns,
        });
    }
    SchemaBlockV2 { groups }
}

pub struct BinaryTelemetryWriterV2 {
    buffer: Vec<TelemetryFrame>,
    chunk_rows: usize,
    schema: SchemaBlockV2,
    metadata: SessionMetadata,
    writer: BufWriter<File>,
    row_group_count: u32,
    total_frames: u64,
    skip_entries: Vec<SkipIndexEntry>,
    lap_entries: Vec<LapIndexEntryV2>,
    current_lap: Option<i32>,
    lap_start_tick: u64,
    lap_sample_count: u32,
    current_lap_is_valid: bool,
    current_lap_is_out_lap: bool,
    schema_bytes: Vec<u8>,

    first_row_group_offset: u64,
}

impl BinaryTelemetryWriterV2 {
    pub fn create_file(
        path: impl AsRef<Path>,
        metadata: SessionMetadata,
        config: LiveTelemetryConfig,
    ) -> TelemetryResult<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path.as_ref())?;
        let mut writer = BufWriter::new(file);
        let schema = build_default_schema();
        let mut schema_buf = Vec::new();
        schema.write_to(&mut schema_buf)?;
        let meta_buf = encode_metadata(&metadata);
        let hdr_size = HEADER_V2_SIZE as u64;
        let schema_offset = hdr_size;
        let metadata_offset = schema_offset + schema_buf.len() as u64;
        let first_rg_offset = metadata_offset + meta_buf.len() as u64;
        let header = FileHeaderV2 {
            schema_offset,
            metadata_offset,
            first_row_group_offset: first_rg_offset,
            footer_offset: 0,
            created_unix_ns: metadata.created_unix_ns,
            poll_hz: metadata.poll_hz as u32,
        };
        header.write_to(&mut writer)?;
        writer.write_all(&schema_buf)?;
        writer.write_all(&meta_buf)?;
        writer.flush()?;
        Ok(Self {
            buffer: Vec::with_capacity(config.chunk_rows),
            chunk_rows: config.chunk_rows,
            schema,
            metadata,
            writer,
            row_group_count: 0,
            total_frames: 0,
            skip_entries: Vec::new(),
            lap_entries: Vec::new(),
            current_lap: None,
            lap_start_tick: 0,
            lap_sample_count: 0,
            current_lap_is_valid: false,
            current_lap_is_out_lap: false,
            schema_bytes: schema_buf,
            first_row_group_offset: first_rg_offset,
        })
    }

    pub fn write_frame(&mut self, frame: &TelemetryFrame) -> TelemetryResult<()> {
        let lap = frame.session.completed_laps + 1;
        let tick = frame.sample_tick;
        let is_valid = frame.session.is_valid_lap != 0;
        match self.current_lap {
            None => {
                self.current_lap = Some(lap);
                self.lap_start_tick = tick;
                self.lap_sample_count = 1;
                self.current_lap_is_valid = is_valid;
                self.current_lap_is_out_lap = lap == 1;
            }
            Some(cur) if cur != lap => {
                self.lap_entries.push(LapIndexEntryV2 {
                    lap_number: cur,
                    start_tick: self.lap_start_tick,
                    end_tick: tick - 1,
                    sample_count: self.lap_sample_count,
                    is_valid: self.current_lap_is_valid as i32,
                    is_out_lap: self.current_lap_is_out_lap as i32,
                });
                self.current_lap = Some(lap);
                self.lap_start_tick = tick;
                self.lap_sample_count = 1;
                self.current_lap_is_valid = is_valid;
                self.current_lap_is_out_lap = lap == 1;
            }
            Some(_) => {
                self.lap_sample_count += 1;
                self.current_lap_is_valid = is_valid;
            }
        }
        self.buffer.push(frame.clone());
        self.total_frames += 1;
        if self.buffer.len() >= self.chunk_rows {
            self.flush_row_group()?;
        }
        Ok(())
    }

    /// Flush any buffered frames to disk and sync the underlying file.
    pub fn flush(&mut self) -> TelemetryResult<()> {
        if !self.buffer.is_empty() {
            self.flush_row_group()?;
        }
        self.writer.flush()?;
        Ok(())
    }

    pub fn finish(mut self) -> TelemetryResult<RecordingSummary> {
        if !self.buffer.is_empty() {
            self.flush_row_group()?;
        }
        if let Some(lap) = self.current_lap {
            let last_tick = self.lap_start_tick + self.lap_sample_count as u64 - 1;
            self.lap_entries.push(LapIndexEntryV2 {
                lap_number: lap,
                start_tick: self.lap_start_tick,
                end_tick: last_tick,
                sample_count: self.lap_sample_count,
                is_valid: self.current_lap_is_valid as i32,
                is_out_lap: self.current_lap_is_out_lap as i32,
            });
        }
        let footer_offset = self.writer.get_ref().metadata()?.len();
        let footer = FooterV2 {
            footer_offset,
            skip_index_count: self.skip_entries.len() as u32,
            lap_index_count: self.lap_entries.len() as u32,
        };
        footer.write_to(&mut self.writer)?;
        footer.write_skip_entries(&mut self.writer, &self.skip_entries)?;
        footer.write_lap_entries(&mut self.writer, &self.lap_entries)?;
        let file = self
            .writer
            .into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let total_bytes = file.metadata()?.len();
        let mut file = file;
        file.seek(SeekFrom::Start(0))?;
        let updated_header = FileHeaderV2 {
            schema_offset: HEADER_V2_SIZE as u64,
            metadata_offset: HEADER_V2_SIZE as u64 + self.schema_bytes.len() as u64,
            first_row_group_offset: self.first_row_group_offset,
            footer_offset,
            created_unix_ns: self.metadata.created_unix_ns,
            poll_hz: self.metadata.poll_hz as u32,
        };
        updated_header.write_to(&mut file)?;
        file.flush()?;
        Ok(RecordingSummary {
            total_samples: self.total_frames,
            chunk_count: self.row_group_count,
            total_bytes,
            footer_offset,
            duration: Duration::from_secs(0),
        })
    }

    fn flush_row_group(&mut self) -> TelemetryResult<()> {
        let frames = std::mem::take(&mut self.buffer);
        let frame_count = frames.len();
        if frame_count == 0 {
            return Ok(());
        }
        let frame_start_tick = frames[0].sample_tick;
        let frame_end_tick = frames[frame_count - 1].sample_tick;
        let rg_index = self.row_group_count;
        let group_order: [GroupId; 7] = [
            GroupId::FrameMeta,
            GroupId::DriverInputs,
            GroupId::VehicleDynamics,
            GroupId::Tyres,
            GroupId::Timing,
            GroupId::Environment,
            GroupId::ColdStorage,
        ];
        let mut group_buffers: HashMap<GroupId, Vec<u8>> = HashMap::new();
        for &group_id in &group_order {
            let group_def = match self
                .schema
                .groups
                .iter()
                .find(|g| g.group_id == group_id as u16)
            {
                Some(g) => g,
                None => continue,
            };
            let mut col_entries: Vec<ColumnEntryV2> = Vec::new();
            let mut col_data: Vec<Vec<u8>> = Vec::new();
            for col_def in &group_def.columns {
                let mut all_values: Vec<f64> = Vec::new();
                let mut value_type: u8 = 0;
                let mut sub_count: u8 = 0;
                for frame in &frames {
                    let (vals, vt) = extract_column_values(frame, col_def.column_id);
                    value_type = vt;
                    if vt == TYPE_BYTES
                        || vt == TYPE_BYTES_F32
                        || vt == TYPE_BYTES_U16
                        || vt == TYPE_BYTES_I32
                    {
                        if sub_count == 0 {
                            sub_count = vals.len() as u8;
                        }
                        all_values.extend(vals);
                    } else {
                        all_values.extend(vals);
                    }
                }
                let codec = CODEC_PLAIN;
                let (encoded, crc32, min_val, max_val) =
                    encode_column(&all_values, value_type, codec, sub_count);
                col_entries.push(ColumnEntryV2 {
                    column_id: col_def.column_id,
                    codec,
                    value_type,
                    byte_len: encoded.len() as u32,
                    crc32,
                    min_value: min_val,
                    max_value: max_val,
                });
                col_data.push(encoded);
            }
            let mut buf = Vec::new();
            buf.extend_from_slice(&(group_id as u16).to_le_bytes());
            buf.extend_from_slice(&(col_entries.len() as u16).to_le_bytes());
            for entry in &col_entries {
                entry.write_to(&mut buf)?;
            }
            for data in &col_data {
                buf.extend_from_slice(data);
            }
            group_buffers.insert(group_id, buf);
        }
        let hdr_entry_count = group_order.len();
        let header_byte_len =
            RowGroupHeader::FIXED_SIZE + hdr_entry_count * RowGroupHeader::ENTRY_SIZE;
        let mut offset = header_byte_len as u32;
        let mut group_entries: Vec<GroupEntryV2> = Vec::new();
        for &group_id in &group_order {
            if let Some(buf) = group_buffers.get(&group_id) {
                group_entries.push(GroupEntryV2 {
                    group_id: group_id as u16,
                    offset,
                    byte_len: buf.len() as u32,
                });
                offset += buf.len() as u32;
            }
        }
        let rg_header = RowGroupHeader {
            row_count: frame_count as u32,
            frame_start_tick,
            frame_end_tick,
            groups: group_entries,
        };
        rg_header.write_to(&mut self.writer)?;
        for &group_id in &group_order {
            if let Some(buf) = group_buffers.get(&group_id) {
                let group_def = match self
                    .schema
                    .groups
                    .iter()
                    .find(|g| g.group_id == group_id as u16)
                {
                    Some(g) => g,
                    None => continue,
                };
                let header_size = 4 + group_def.columns.len() * COLUMN_ENTRY_V2_SIZE;
                let mut data_offset = header_size;
                for col_def in &group_def.columns {
                    let count = u16::from_le_bytes([buf[2], buf[3]]) as usize;
                    let mut pos: usize = 4;
                    let mut found: Option<ColumnEntryV2> = None;
                    for _ in 0..count {
                        let mut cursor = std::io::Cursor::new(&buf[pos..]);
                        if let Ok(entry) = ColumnEntryV2::read_from(&mut cursor) {
                            if entry.column_id == col_def.column_id {
                                found = Some(entry);
                                break;
                            }
                        }
                        pos += COLUMN_ENTRY_V2_SIZE;
                    }
                    if let Some(col_entry) = found {
                        self.skip_entries.push(SkipIndexEntry {
                            access_group: group_id as u16,
                            column_id: col_def.column_id,
                            frame_start: frame_start_tick,
                            frame_end: frame_end_tick,
                            row_group_index: rg_index,
                            offset_in_group: data_offset as u32,
                            byte_len: col_entry.byte_len,
                        });
                        data_offset += col_entry.byte_len as usize;
                    }
                }
            }
        }
        for &group_id in &group_order {
            if let Some(buf) = group_buffers.get(&group_id) {
                self.writer.write_all(buf)?;
            }
        }
        self.writer.flush()?;
        self.row_group_count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    // use std::io::Cursor; // unused

    fn test_frame(t: u64, lap: u32) -> TelemetryFrame {
        let ns = t * 8_333_333;
        let a4 = |b: f32, s: f32| -> [f32; 4] {
            [
                b + s * (t % 4) as f32,
                b + s * ((t + 1) % 4) as f32,
                b + s * ((t + 2) % 4) as f32,
                b + s * ((t + 3) % 4) as f32,
            ]
        };
        TelemetryFrame {
            sample_tick: t,
            timestamp_ns: ns,
            controls: ControlSample {
                sample_tick: t,
                timestamp_ns: ns,
                physics_packet_id: (t as i32).wrapping_mul(7),
                graphics_packet_id: (t as i32).wrapping_mul(13),
                speed_kmh: 150.0 + t as f32 * 0.1,
                gas: ((t % 100) as f32) / 100.0,
                brake: (((t + 50) % 100) as f32) / 120.0,
                clutch: ((t % 10) as f32) / 100.0,
                steer_angle: t as f32 * 0.02,
                gear: 1 + (t % 6) as i32,
                rpms: 5000 + (t as i32) * 50,
                fuel: 80.0 - t as f32 * 0.05,
            },
            motion: MotionSample {
                sample_tick: t,
                timestamp_ns: ns,
                velocity: [t as f32 * 0.5, t as f32 * 0.3, t as f32 * 0.1],
                acc_g: [t as f32 * 0.01, t as f32 * 0.02, -9.81],
                local_velocity: [t as f32 * 0.8, t as f32 * 0.02, t as f32 * 0.005],
                local_angular_vel: [t as f32 * 0.001, t as f32 * 0.002, t as f32 * 0.0005],
                heading: t as f32 * 0.05,
                pitch: t as f32 * 0.01,
                roll: t as f32 * 0.02,
            },
            tyres: TyreSample {
                sample_tick: t,
                timestamp_ns: ns,
                wheel_slip: a4(0.01, 0.005),
                wheel_load: a4(2000.0, 50.0),
                wheels_pressure: a4(27.0, 0.5),
                wheel_angular_speed: a4(80.0, 5.0),
                tyre_wear: a4(95.0, -0.5),
                tyre_dirty_level: a4(0.0, 0.1),
                tyre_core_temperature: a4(80.0, 2.0),
                camber_rad: a4(-0.02, 0.001),
                suspension_travel: a4(0.02, 0.005),
                slip_ratio: a4(0.0, 0.01),
                slip_angle: a4(0.0, 0.02),
                tyre_temp_i: a4(75.0, 2.0),
                tyre_temp_m: a4(80.0, 2.0),
                tyre_temp_o: a4(85.0, 2.0),
                tyre_temp: a4(82.0, 2.0),
                mz: a4(0.0, 10.0),
                fx: a4(0.0, 100.0),
                fy: a4(0.0, 50.0),
                suspension_damage: a4(0.0, 1.0),
                brake_temp: a4(400.0, 20.0),
                brake_pressure: a4(10.0, 2.0),
                pad_life: a4(80.0, -1.0),
                disc_life: a4(90.0, -1.0),
                tyre_contact_point: [0.0f32; 12],
                tyre_contact_normal: [0.0f32; 12],
                tyre_contact_heading: [0.0f32; 12],
                number_of_tyres_out: 0,
                front_brake_compound: 0,
                rear_brake_compound: 0,
            },
            powertrain: PowertrainSample {
                sample_tick: t,
                timestamp_ns: ns,
                turbo_boost: 1.2 + t as f32 * 0.01,
                ballast: 0.0,
                kers_charge: 80.0 - t as f32 * 0.1,
                kers_input: t as f32 * 0.5,
                kers_current_kj: 1000.0 + t as f32,
                drs: if t % 2 == 0 { 1.0 } else { 0.0 },
                tc: 2.0 + (t % 5) as f32 * 0.1,
                abs: 3.0 - (t % 4) as f32 * 0.1,
                engine_brake: (t % 8) as i32,
                ers_recovery_level: (t % 4) as i32,
                ers_power_level: (t % 3) as i32,
                ers_heat_charging: (t % 2) as i32,
                ers_is_charging: (t % 2) as i32,
                drs_available: 1,
                drs_enabled: (t % 2) as i32,
                tc_in_action: (t % 3) as i32,
                abs_in_action: (t % 3) as i32,
                auto_shifter_on: 1,
                current_max_rpm: 8000,
                p2p_activations: (t % 5) as i32,
                p2p_status: (t % 2) as i32,
                water_temp: 90.0 + t as f32 * 0.1,
            },
            session: SessionSample {
                sample_tick: t,
                timestamp_ns: ns,
                status: 2,
                session: 0,
                session_index: 0,
                completed_laps: lap as i32 - 1,
                position: 1 + (t % 10) as i32,
                session_time_left: 1800.0 - t as f32 * 10.0,
                number_of_laps: 15,
                current_sector_index: 1 + (t % 3) as i32,
                normalized_car_position: t as f32 * 0.02,
                is_in_pit: 0,
                is_in_pit_lane: 0,
                mandatory_pit_done: 0,
                missing_mandatory_pits: 0,
                penalty_time: 0.0,
                penalty_type: 0,
                track_status: [0u16; 33],
                clock: t as f32 * 10.0,
                replay_time_multiplier: 1.0,
                is_valid_lap: 1,
                global_yellow: 0,
                global_yellow1: 0,
                global_yellow2: 0,
                global_yellow3: 0,
                global_white: 0,
                global_green: 1,
                global_chequered: 0,
                global_red: 0,
                gap_ahead_or_tail_value: 0,
                flag: 0,
                gap_behind: 0,
            },
            timing: TimingSample {
                sample_tick: t,
                timestamp_ns: ns,
                i_current_time: (t as i32) * 1000,
                i_last_time: (t as i32) * 995,
                i_best_time: (t as i32) * 990,
                i_split: (t as i32) * 330,
                last_sector_time: (t as i32) * 330,
                i_delta_lap_time: (t as i32).wrapping_mul(5),
                is_delta_positive: if t % 2 == 0 { 0 } else { 1 },
                i_estimated_lap_time: (t as i32) * 980,
                fuel_estimated_laps: 10.0 - t as f32 * 0.01,
                fuel_x_lap: 2.5 + t as f32 * 0.001,
                used_fuel: t as f32 * 0.05,
                distance_traveled: t as f32 * 10.0,
                current_time_str: [0u16; 15],
                last_time_str: [0u16; 15],
                best_time_str: [0u16; 15],
                split_str: [0u16; 15],
                delta_lap_time_str: [0u16; 15],
                estimated_lap_time_str: [0u16; 15],
                observed_slot_before_i_split: 0,
            },
            car_state: CarStateSample {
                sample_tick: t,
                timestamp_ns: ns,
                car_damage: [0.0f32; 5],
                pit_limiter_on: 0,
                ride_height: [0.1, 0.12],
                ignition_on: 1,
                starter_engine_on: 1,
                is_engine_running: 1,
                is_ai_controlled: 0,
                cg_height: 0.35,
                brake_bias: 55.0,
                rain_lights: 0,
                flashing_lights: 0,
                lights_stage: 0,
                wiper_lv: 0,
                driver_stint_total_time_left: 3600,
                driver_stint_time_left: 1800,
                rain_tyres: 0,
                current_tyre_set: 0,
                strategy_tyre_set: 0,
                track_grip_status: 2,
                tyre_compound_str: [0u16; 33],
                mfd_tyre_set: 0,
                mfd_fuel_to_add: 0.0,
                mfd_tyre_pressure: [27.0; 4],
                ideal_line_on: 0,
                is_setup_menu_visible: 0,
                main_display_index: 0,
                secondary_display_index: 0,
                direction_lights_left: 0,
                direction_lights_right: 0,
                tc_level: 2,
                tc_cut: 0,
                engine_map: 1,
                abs_level: 3,
                exhaust_temperature: 600.0,
                final_ff: 0.0,
                performance_meter: 0.0,
                kerb_vibration: 0.0,
                slip_vibrations: 0.0,
                g_vibrations: 0.0,
                abs_vibrations: 0.0,
            },
            environment: EnvironmentSample {
                sample_tick: t,
                timestamp_ns: ns,
                air_density: 1.2,
                air_temp: 25.0,
                road_temp: 35.0,
                wind_speed: 5.0,
                wind_direction: 180.0,
                surface_grip: 0.95,
                rain_intensity: 0,
                rain_intensity_in_10min: 0,
                rain_intensity_in_30min: 0,
            },
            other_cars: OtherCarsSample {
                sample_tick: t,
                timestamp_ns: ns,
                active_cars: 24,
                player_car_id: 1,
                car_coordinates: vec![0.0f32; 180],
                car_id: vec![0i32; 60],
            },
        }
    }

    fn test_meta() -> SessionMetadata {
        SessionMetadata {
            track_name: "test".into(),
            car_model: "test".into(),
            created_unix_ns: 1_700_000_000_000_000_000,
            poll_hz: 120.0,
            chunk_rows: 1024,
            sm_version: "sm".into(),
            ac_version: "ac".into(),
            number_of_sessions: 1,
            num_cars: 24,
            sector_count: 3,
            max_rpm: 9000,
            max_torque: 650.0,
            max_power: 700.0,
            max_fuel: 100.0,
            penalties_enabled: 1,
            raw_static_bytes: vec![0u8; 1024],
            session_type: Some(9),
        }
    }

    #[test]
    fn test_create_and_write() {
        let tmp = std::env::temp_dir().join(format!("v2_test_{}.acctlm2", std::process::id()));
        let cfg = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 1024,
        };
        let mut w = BinaryTelemetryWriterV2::create_file(&tmp, test_meta(), cfg).unwrap();
        w.write_frame(&test_frame(0, 1)).unwrap();
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_write_and_finish() {
        let tmp = std::env::temp_dir().join(format!("v2_finish_{}.acctlm2", std::process::id()));
        let cfg = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 3,
        };
        let mut w = BinaryTelemetryWriterV2::create_file(&tmp, test_meta(), cfg).unwrap();
        for t in 0..5 {
            w.write_frame(&test_frame(t, 1)).unwrap();
        }
        let s = w.finish().unwrap();
        assert!(s.total_samples >= 5);
        assert!(s.total_bytes > 0);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_lap_tracking() {
        let tmp = std::env::temp_dir().join(format!("v2_laps_{}.acctlm2", std::process::id()));
        let cfg = LiveTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 10,
        };
        let mut w = BinaryTelemetryWriterV2::create_file(&tmp, test_meta(), cfg).unwrap();
        for lap in 1..=3 {
            for t in 0..5 {
                let tick = (lap - 1) * 5 + t;
                let mut f = test_frame(tick, lap as u32);
                f.session.completed_laps = (lap - 1) as i32;
                w.write_frame(&f).unwrap();
            }
        }
        w.finish().unwrap();
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_extract_scalar() {
        let f = test_frame(0, 1);
        let (v, t) = extract_column_values(&f, 1);
        assert_eq!(v, vec![0.0]);
        assert_eq!(t, TYPE_U64);
    }

    #[test]
    fn test_extract_velocity() {
        let f = test_frame(5, 1);
        let (v, t) = extract_column_values(&f, 20);
        assert_eq!(v, vec![2.5, 1.5, 0.5]);
        assert_eq!(t, TYPE_BYTES_F32);
    }
}
