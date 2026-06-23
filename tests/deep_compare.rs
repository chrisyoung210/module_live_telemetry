use module_live_telemetry::BinaryTelemetryReader;
use std::path::Path;

#[test]
fn deep_compare_v1_v2() {
    let v1_path = Path::new("data/session4.acctlm");
    let v2_path = Path::new("data/session4.acctlm2");

    println!("Reading v1...");
    let v1 = BinaryTelemetryReader::open(v1_path).unwrap();
    let v1_frames = v1.read_all_frames().unwrap();
    println!("  {} frames", v1_frames.len());

    println!("Reading v2...");
    let v2 = BinaryTelemetryReader::open(v2_path).unwrap();
    let v2_frames = v2.read_all_frames().unwrap();
    println!("  {} frames", v2_frames.len());

    assert_eq!(v1_frames.len(), v2_frames.len(), "frame count mismatch");

    let mut mismatches = 0u64;
    for i in 0..v1_frames.len() {
        let a = &v1_frames[i];
        let b = &v2_frames[i];

        macro_rules! check {
            ($struct:ident, $field:ident) => {
                if a.$struct.$field != b.$struct.$field {
                    mismatches += 1;
                    if mismatches <= 10 {
                        eprintln!(
                            "frame[{}] {}.{}: v1={:?} v2={:?}",
                            i,
                            stringify!($struct),
                            stringify!($field),
                            a.$struct.$field,
                            b.$struct.$field
                        );
                    }
                }
            };
            ($struct:ident, $field:ident, $idx:expr) => {
                if a.$struct.$field[$idx] != b.$struct.$field[$idx] {
                    mismatches += 1;
                    if mismatches <= 10 {
                        eprintln!(
                            "frame[{}] {}.{}[{}]: v1={:?} v2={:?}",
                            i,
                            stringify!($struct),
                            stringify!($field),
                            $idx,
                            a.$struct.$field[$idx],
                            b.$struct.$field[$idx]
                        );
                    }
                }
            };
        }

        // Controls
        check!(controls, sample_tick);
        check!(controls, timestamp_ns);
        check!(controls, physics_packet_id);
        check!(controls, graphics_packet_id);
        check!(controls, speed_kmh);
        check!(controls, gas);
        check!(controls, brake);
        check!(controls, clutch);
        check!(controls, steer_angle);
        check!(controls, gear);
        check!(controls, rpms);
        check!(controls, fuel);

        // Motion
        check!(motion, velocity, 0);
        check!(motion, velocity, 1);
        check!(motion, velocity, 2);
        check!(motion, acc_g, 0);
        check!(motion, acc_g, 1);
        check!(motion, acc_g, 2);
        check!(motion, heading);
        check!(motion, pitch);
        check!(motion, roll);

        // Tyres (sample: wheel_slip[0], wheel_load[0], wheels_pressure[0])
        check!(tyres, wheel_slip, 0);
        check!(tyres, wheel_load, 0);
        check!(tyres, wheels_pressure, 0);
        check!(tyres, tyre_core_temperature, 0);
        check!(tyres, camber_rad, 0);

        // Powertrain
        check!(powertrain, turbo_boost);
        check!(powertrain, kers_charge);
        check!(powertrain, drs);
        check!(powertrain, tc);
        check!(powertrain, abs);

        // Session
        check!(session, status);
        check!(session, session_index);
        check!(session, completed_laps);
        check!(session, position);
        check!(session, current_sector_index);
        check!(session, normalized_car_position);
        check!(session, is_in_pit);
        check!(session, is_valid_lap);
        check!(session, penalty_time);
        check!(session, clock);
        check!(session, replay_time_multiplier);

        // Timing
        check!(timing, i_current_time);
        check!(timing, i_last_time);
        check!(timing, i_best_time);
        check!(timing, last_sector_time);
        check!(timing, i_delta_lap_time);

        // CarState
        check!(car_state, cg_height);
        check!(car_state, brake_bias);

        // Environment
        check!(environment, air_temp);
        check!(environment, road_temp);
        check!(environment, wind_speed);
        check!(environment, rain_intensity);

        // OtherCars
        check!(other_cars, active_cars);
    }

    if mismatches == 0 {
        println!(
            "ALL {} frames match perfectly across all fields.",
            v1_frames.len()
        );
    } else {
        println!("ERROR: {} total mismatches found.", mismatches);
        panic!("{} mismatches found", mismatches);
    }
}
