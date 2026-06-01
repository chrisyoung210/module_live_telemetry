use module_live_telemetry::{
    mock::generate_mock_controls, shmem::AccGameStatus, shmem::AccSharedMemoryReader,
    BinaryTelemetryReader, BinaryTelemetryWriter, ControlSample, LiveTelemetryConfig,
    SessionMetadata, SessionSample, TelemetryError, TelemetryFrame, TelemetryResult,
    TimingSample,
};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> TelemetryResult<()> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };
    let args: Vec<String> = args.collect();
    match command.as_str() {
        "generate-mock" => generate_mock_command(&args),
        "record" => record_command(&args),
        "inspect" => inspect_command(&args),
        "export" => export_command(&args),
        "laps" => laps_command(&args),
        "restore" => restore_command(&args),
        "import-raw" => import_raw_command(&args),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => Err(TelemetryError::InvalidArgument(format!(
            "unknown command '{command}'"
        ))),
    }
}

fn generate_mock_command(args: &[String]) -> TelemetryResult<()> {
    let out = required_path(args, "--out")?;
    let samples = optional_usize(args, "--samples", 1_000)?;
    let poll_hz = optional_f64(args, "--poll-hz", 120.0)?;
    let chunk_rows = optional_usize(args, "--chunk-rows", 256)?;
    let track = optional_string(args, "--track", "monza");
    let car = optional_string(args, "--car", "mclaren_720s_gt3_evo");

    let metadata = SessionMetadata::new(track, car, poll_hz);
    let config = LiveTelemetryConfig {
        poll_hz,
        chunk_rows,
    };
    let mut writer = BinaryTelemetryWriter::create_file(&out, metadata, config)?;
    for sample in generate_mock_controls(samples, poll_hz) {
        let frame = TelemetryFrame {
            sample_tick: sample.sample_tick,
            timestamp_ns: sample.timestamp_ns,
            controls: sample,
            motion: Default::default(),
            tyres: Default::default(),
            powertrain: Default::default(),
            session: Default::default(),
            timing: Default::default(),
            car_state: Default::default(),
            environment: Default::default(),
            other_cars: Default::default(),
        };
        writer.write_frame(frame)?;
    }
    let (_file, summary) = writer.finish()?;
    println!(
        "wrote {} samples in {} chunks to {} ({} bytes)",
        summary.total_samples,
        summary.chunk_count,
        out.display(),
        summary.total_bytes
    );
    Ok(())
}

fn record_command(args: &[String]) -> TelemetryResult<()> {
    let out = optional_path(args, "--out");
    let out_dir = optional_path(args, "--out-dir").unwrap_or_else(|| PathBuf::from(".\\data"));
    let poll_hz = optional_f64(args, "--poll-hz", 120.0)?;
    let chunk_rows = optional_usize(args, "--chunk-rows", 256)?;
    let status_interval_ms = optional_u64(args, "--status-interval-ms", 2000)?;
    let flush_interval_ms = optional_u64(args, "--flush-interval-ms", 2000)?;
    let poll_interval = Duration::from_secs_f64(1.0 / poll_hz.max(1.0));
    let status_interval = Duration::from_millis(status_interval_ms.max(250));
    let flush_interval = if flush_interval_ms == 0 {
        None
    } else {
        Some(Duration::from_millis(flush_interval_ms.max(250)))
    };

    ensure_parent_dir(out.as_deref().unwrap_or(&out_dir))?;
    if out.is_none() {
        std::fs::create_dir_all(&out_dir)?;
    }

    println!("waiting for ACC shared memory...");
    let mut reader: Option<AccSharedMemoryReader> = None;
    let mut writer: Option<BinaryTelemetryWriter<File>> = None;
    let mut output_path: Option<PathBuf> = None;
    let mut sample_tick = 0u64;
    let mut recording_started_at = Instant::now();
    let mut last_status_log = Instant::now() - status_interval;
    let mut last_flush = Instant::now();
    let mut last_status: Option<AccGameStatus> = None;

    loop {
        let tick_start = Instant::now();

        if reader.is_none() {
            match AccSharedMemoryReader::open() {
                Ok(opened) => {
                    println!("connected to ACC shared memory");
                    reader = Some(opened);
                    last_status = None;
                }
                Err(err) => {
                    if last_status_log.elapsed() >= status_interval {
                        println!("ACC not available yet: {err}");
                        last_status_log = Instant::now();
                    }
                    sleep_remaining(tick_start, poll_interval);
                    continue;
                }
            }
        }

        let active_reader = reader.as_mut().expect("reader exists");
        let status = match active_reader.status() {
            Ok(status) => status,
            Err(err) => {
                if let Some(active_writer) = writer.take() {
                    let (_, summary) = active_writer.finish()?;
                    println!(
                        "shared memory disappeared; finished recording {} samples in {} chunks",
                        summary.total_samples, summary.chunk_count
                    );
                    return Ok(());
                }
                if last_status_log.elapsed() >= status_interval {
                    println!("lost ACC shared memory before recording: {err}");
                    last_status_log = Instant::now();
                }
                reader = None;
                sleep_remaining(tick_start, poll_interval);
                continue;
            }
        };

        if Some(status) != last_status {
            println!("ACC status: {}", status.label());
            last_status = Some(status);
        }

        if status.is_live() {
            if writer.is_none() {
                let session = active_reader.session_info();
                let path = match &out {
                    Some(path) => path.clone(),
                    None => out_dir.join(default_recording_name(
                        &session.track_name,
                        &session.car_model,
                    )),
                };
                ensure_parent_dir(&path)?;
                let metadata = SessionMetadata::new(
                    session.track_name,
                    session.car_model,
                    poll_hz,
                );
                let config = LiveTelemetryConfig {
                    poll_hz,
                    chunk_rows,
                };
                writer = Some(BinaryTelemetryWriter::create_file(&path, metadata, config)?);
                output_path = Some(path.clone());
                sample_tick = 0;
                recording_started_at = Instant::now();
                last_flush = Instant::now();
                println!("recording started: {}", path.display());
            }

            let timestamp_ns = recording_started_at
                .elapsed()
                .as_nanos()
                .min(u64::MAX as u128) as u64;
            match active_reader.read_telemetry_frame(sample_tick, timestamp_ns) {
                Ok(Some(frame)) => {
                    if let Some(active_writer) = writer.as_mut() {
                        active_writer.write_frame(frame)?;
                    }
                    sample_tick = sample_tick.saturating_add(1);
                }
                Ok(None) => {}
                Err(err) => {
                    if let Some(active_writer) = writer.take() {
                        let (_, summary) = active_writer.finish()?;
                        println!(
                            "read failed; finished recording {} samples in {} chunks: {}",
                            summary.total_samples, summary.chunk_count, err
                        );
                        if let Some(path) = &output_path {
                            println!("output: {}", path.display());
                        }
                        return Ok(());
                    }
                    return Err(err);
                }
            }
        } else if status.is_pause() {
            if writer.is_some() && last_status_log.elapsed() >= status_interval {
                println!("ACC paused; recording is kept open and sampling is suspended");
                last_status_log = Instant::now();
            }
        } else if let Some(active_writer) = writer.take() {
            let (_, summary) = active_writer.finish()?;
            println!(
                "session ended; finished recording {} samples in {} chunks",
                summary.total_samples, summary.chunk_count
            );
            if let Some(path) = &output_path {
                println!("output: {}", path.display());
            }
            return Ok(());
        } else if last_status_log.elapsed() >= status_interval {
            println!(
                "waiting for live session, current status: {}",
                status.label()
            );
            last_status_log = Instant::now();
        }

        if let (Some(interval), Some(active_writer)) = (flush_interval, writer.as_mut()) {
            if last_flush.elapsed() >= interval {
                active_writer.flush_to_disk()?;
                last_flush = Instant::now();
            }
        }

        sleep_remaining(tick_start, poll_interval);
    }
}

fn inspect_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let reader = BinaryTelemetryReader::open(&input)?;
    let metadata = reader.metadata();
    let summary = reader.summary();
    println!("file: {}", input.display());
    println!("format: ACTL v{}", reader.header().version);
    println!("track: {}", metadata.track_name);
    println!("car: {}", metadata.car_model);
    println!("poll_hz: {:.3}", metadata.poll_hz);
    println!("sm_version: {}", metadata.sm_version);
    println!("ac_version: {}", metadata.ac_version);
    println!("chunks: {}", summary.chunk_count);
    println!("samples: {}", summary.total_samples);
    println!("bytes: {}", summary.total_bytes);
    println!("footer_offset: {}", summary.footer_offset);
    for entry in reader.chunk_index() {
        println!(
            "chunk #{:04} cluster=0x{:04x} samples={} ticks={}..{} time_ns={}..{} offset={} bytes={}",
            entry.chunk_seq,
            entry.cluster_id,
            entry.end_tick.saturating_sub(entry.start_tick).saturating_add(1),
            entry.start_tick,
            entry.end_tick,
            entry.start_time_ns,
            entry.end_time_ns,
            entry.file_offset,
            entry.byte_len
        );
    }
    Ok(())
}

fn export_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let out = optional_path(args, "--out");
    let format = optional_string(args, "--format", "csv");
    if format != "csv" {
        return Err(TelemetryError::InvalidArgument(
            "only --format csv is supported".to_string(),
        ));
    }

    let reader = BinaryTelemetryReader::open(input)?;
    let samples = reader.read_all_controls()?;
    match out {
        Some(path) => {
            let file = File::create(path)?;
            let mut writer = BufWriter::new(file);
            write_csv(&mut writer, &samples)?;
            writer.flush()?;
        }
        None => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            write_csv(&mut lock, &samples)?;
        }
    }
    Ok(())
}

fn write_csv(writer: &mut impl Write, samples: &[ControlSample]) -> TelemetryResult<()> {
    writeln!(writer, "{}", ControlSample::csv_header())?;
    for sample in samples {
        writeln!(writer, "{}", sample.to_csv_row())?;
    }
    Ok(())
}

// ---- Helpers ----

fn default_recording_name(_track: &str, car: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    PathBuf::from(format!("live_{}_{}.acctlm", timestamp, car))
}

fn ensure_parent_dir(path: &std::path::Path) -> TelemetryResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn sleep_remaining(tick_start: Instant, interval: Duration) {
    let elapsed = tick_start.elapsed();
    if elapsed < interval {
        thread::sleep(interval - elapsed);
    }
}

fn required_path(args: &[String], flag: &str) -> TelemetryResult<PathBuf> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .ok_or_else(|| TelemetryError::InvalidArgument(format!("missing {flag} <path>")))
}

fn optional_path(args: &[String], flag: &str) -> Option<PathBuf> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}

fn optional_string<'a>(args: &'a [String], flag: &str, default: &'a str) -> &'a str {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or(default)
}

fn optional_usize(args: &[String], flag: &str, default: usize) -> TelemetryResult<usize> {
    match args.iter().position(|a| a == flag) {
        Some(i) => args
            .get(i + 1)
            .ok_or_else(|| TelemetryError::InvalidArgument(format!("missing value for {flag}")))?
            .parse::<usize>()
            .map_err(|e| TelemetryError::InvalidArgument(format!("invalid {flag}: {e}"))),
        None => Ok(default),
    }
}

fn optional_f64(args: &[String], flag: &str, default: f64) -> TelemetryResult<f64> {
    match args.iter().position(|a| a == flag) {
        Some(i) => args
            .get(i + 1)
            .ok_or_else(|| TelemetryError::InvalidArgument(format!("missing value for {flag}")))?
            .parse::<f64>()
            .map_err(|e| TelemetryError::InvalidArgument(format!("invalid {flag}: {e}"))),
        None => Ok(default),
    }
}

fn optional_u64(args: &[String], flag: &str, default: u64) -> TelemetryResult<u64> {
    match args.iter().position(|a| a == flag) {
        Some(i) => args
            .get(i + 1)
            .ok_or_else(|| TelemetryError::InvalidArgument(format!("missing value for {flag}")))?
            .parse::<u64>()
            .map_err(|e| TelemetryError::InvalidArgument(format!("invalid {flag}: {e}"))),
        None => Ok(default),
    }
}

fn laps_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let reader = BinaryTelemetryReader::open(&input)?;
    let metadata = reader.metadata();
    let summary = reader.summary();

    println!("file: {}", input.display());
    println!("track: {}", metadata.track_name);
    println!("car: {}", metadata.car_model);
    println!("poll_hz: {:.3}", metadata.poll_hz);
    println!("total_samples: {}", summary.total_samples);
    println!();

    // Read session samples to get lap boundaries
    let session_samples = reader.read_all_session().unwrap_or_default();
    let timing_samples = reader.read_all_timing().unwrap_or_default();

    if session_samples.is_empty() && timing_samples.is_empty() {
        println!("no session or timing data found in this file");
        return Ok(());
    }

    // Aggregate lap data from session and timing samples.
    // Detection strategy:
    //   1. `current_sector_index` resetting (from 1+ to 0) indicates crossing start/finish line.
    //   2. `completed_laps` incrementing also indicates a line crossing.
    //   3. We merge these signals: a new lap starts when EITHER signal fires.
    // This correctly separates the out lap from the first timed lap.
    #[derive(Debug)]
    struct LapInfo {
        lap_number: i32,
        start_tick: u64,
        end_tick: u64,
        sample_count: usize,
        is_valid: bool,
        is_out_lap: bool,
        i_last_time_ms: Option<i32>,
        i_best_time_ms: Option<i32>,
    }

    if !session_samples.is_empty() {
        // --- Lap boundary detection ---
        //
        // Use `normalized_car_position` (0.0–1.0, wraps at S/F line).
        // When it drops from ~0.9+ to ~0.1- the car crossed start/finish.

        let mut crossings: Vec<usize> = Vec::new();
        for i in 1..session_samples.len() {
            let prev_pos = session_samples[i - 1].normalized_car_position;
            let cur_pos = session_samples[i].normalized_car_position;
            // Position wrapped from near 1.0 back to near 0.0 = lap boundary
if prev_pos > 0.8 && cur_pos < 0.2 {
                crossings.push(i);
            }
        }

        // Build lap segments from crossings (normalized_car_position wraps)
        let timing_ticks: Vec<u64> = timing_samples.iter().map(|t| t.sample_tick).collect();
        let mut laps: Vec<LapInfo> = Vec::new();
        let mut lap_start_idx: usize = 0;
        let mut lap_number: i32 = 0;

        for &cross_idx in &crossings {
            let start_tick = session_samples[lap_start_idx].sample_tick;
            let end_tick = session_samples[cross_idx - 1].sample_tick;
            let sample_count = cross_idx - lap_start_idx;
            let is_out_lap = lap_number == 0;
            // is_valid_lap at the crossing point is the flag for the JUST COMPLETED lap.
            // During the lap it carries a timer value; only at the crossing it resets
            // is_valid_lap drops from timer values (500+) to a flag (0=invalid, >0=valid)
            // at the crossing point. The flag may appear at cross_idx-1 or cross_idx-2.
            // If EITHER position shows 0, the completed lap was invalidated.
            let a = cross_idx.saturating_sub(1);
            let b = cross_idx.saturating_sub(2);
            let valid = !is_out_lap
                && session_samples[a].is_valid_lap != 0
                && session_samples[b].is_valid_lap != 0;

            // Find timing data for this lap: look for i_last_time at or near the crossing point
            let (last_time_ms, best_time_ms) = if !timing_samples.is_empty() {
                let timing_at_cross = timing_ticks
                    .binary_search(&session_samples[cross_idx].sample_tick)
                    .ok()
                    .map(|idx| &timing_samples[idx])
                    .or_else(|| {
                        timing_samples
                            .iter()
                            .filter(|t| t.sample_tick >= start_tick && t.sample_tick <= session_samples[cross_idx].sample_tick)
                            .max_by_key(|t| t.sample_tick)
                    });
                timing_at_cross
                    .map(|t| {
                        let last = if t.i_last_time > 0 && t.i_last_time < 2_000_000 { Some(t.i_last_time) } else { None };
                        let best = if t.i_best_time > 0 && t.i_best_time < 2_000_000 { Some(t.i_best_time) } else { None };
                        (last, best)
                    })
                    .unwrap_or((None, None))
            } else {
                (None, None)
            };

            laps.push(LapInfo {
                lap_number,
                start_tick,
                end_tick,
                sample_count,
                is_valid: valid,
                is_out_lap,
                i_last_time_ms: last_time_ms,
                i_best_time_ms: best_time_ms,
            });
            lap_start_idx = cross_idx;
            lap_number += 1;
        }

// Push remaining samples as the last (possibly incomplete) lap
        if lap_start_idx < session_samples.len() {
            let last_idx = session_samples.len() - 1;
            let start_tick = session_samples[lap_start_idx].sample_tick;
            let end_tick = session_samples[last_idx].sample_tick;
            let sample_count = last_idx - lap_start_idx + 1;
            let is_out_lap = lap_number == 0;
            // Last (incomplete) lap: check the final sample's flag
            let valid = !is_out_lap && session_samples[last_idx].is_valid_lap != 0;

            laps.push(LapInfo {
                lap_number,
                start_tick,
                end_tick,
                sample_count,
                is_valid: valid,
                is_out_lap,
                i_last_time_ms: None,
                i_best_time_ms: None,
            });
        }

        // Display
        let format_lap_time = |ms: Option<i32>| -> String {
            ms.filter(|&v| v > 0 && v < 2_000_000)
                .map(|v| format!("{}:{:02}.{:03}", v / 60000, (v % 60000) / 1000, v % 1000))
                .unwrap_or_else(|| "-".to_string())
        };

        println!("laps: {} total laps recorded", laps.len());
        println!();
        println!(
            "{:<5} {:<5} {:<14} {:<14} {:<10} {:<8} {:<12} {:<12}",
            "lap", "out?", "start_tick", "end_tick", "samples", "valid", "time", "best"
        );

        for lap in &laps {
            println!(
                "{:<5} {:<5} {:<14} {:<14} {:<10} {:<8} {:<12} {:<12}",
                lap.lap_number,
                if lap.is_out_lap { "yes" } else { "" },
                lap.start_tick,
                lap.end_tick,
                lap.sample_count,
                if lap.is_valid { "yes" } else { "no" },
                format_lap_time(lap.i_last_time_ms),
                format_lap_time(lap.i_best_time_ms),
            );
        }

        // No session data; show timing summary instead
    if timing_samples.is_empty() {
            println!("no session data (lap boundaries); showing timing samples summary:");
            println!("timing samples: {}", timing_samples.len());
            if let Some(first) = timing_samples.first() {
                println!("first sample tick: {}", first.sample_tick);
            }
            if let Some(last) = timing_samples.last() {
                println!("last sample tick: {}", last.sample_tick);
            }

            // Show timing samples that have non-zero i_last_time (lap completions)
            let laps_with_times: Vec<_> = timing_samples
                .iter()
                .filter(|t| t.i_last_time > 0)
                .collect();
            if !laps_with_times.is_empty() {
                println!();
                println!("samples with lap times (i_last_time > 0): {}", laps_with_times.len());
                println!(
                    "{:<14} {:<12} {:<12} {:<12}",
                    "tick", "last_ms", "best_ms", "sector_ms"
                );
                for t in laps_with_times.iter().take(50) {
                    println!(
                        "{:<14} {:<12} {:<12} {:<12}",
                        t.sample_tick, t.i_last_time, t.i_best_time, t.last_sector_time
                    );
                }
            }
        }
    }

    Ok(())
}

fn restore_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let out_dir = optional_path(args, "--out-dir").unwrap_or_else(|| PathBuf::from(".\\restored"));
    fs::create_dir_all(&out_dir)?;

    let reader = BinaryTelemetryReader::open(&input)?;
    let meta = reader.metadata();
    let summary = reader.summary();

    // metadata.txt
    let meta_path = out_dir.join("metadata.txt");
    let mut mf = BufWriter::new(File::create(&meta_path)?);
    writeln!(mf, "track={}", meta.track_name)?;
    writeln!(mf, "car={}", meta.car_model)?;
    writeln!(mf, "poll_hz={}", meta.poll_hz)?;
    writeln!(mf, "chunk_rows={}", meta.chunk_rows)?;
    writeln!(mf, "total_samples={}", summary.total_samples)?;
    writeln!(mf, "version={}", reader.header().version)?;
    mf.flush()?;
    println!("wrote {}", meta_path.display());

    // session.csv
    let session_path = out_dir.join("session.csv");
    let mut sf = BufWriter::new(File::create(&session_path)?);
    writeln!(sf, "tick,ns,status,session,sessIdx,complLaps,pos,sessLeft,numLaps,sector,normPos,inPit,inPitLane,mndPit,missPit,penTime,penType,trkStat_0,trkStat_1,trkStat_2,trkStat_3,trkStat_4,trkStat_5,trkStat_6,trkStat_7,trkStat_8,trkStat_9,trkStat_10,trkStat_11,trkStat_12,trkStat_13,trkStat_14,trkStat_15,trkStat_16,trkStat_17,trkStat_18,trkStat_19,trkStat_20,trkStat_21,trkStat_22,trkStat_23,trkStat_24,trkStat_25,trkStat_26,trkStat_27,trkStat_28,trkStat_29,trkStat_30,trkStat_31,trkStat_32,clock,replayMult,isValid,glbY,glbY1,glbY2,glbY3,glbW,glbG,glbC,glbR,gapAhead")?;
    for s in reader.read_all_session().unwrap_or_default() {
        write!(sf, "{},{},{},{},{},{},{},{:.6},{},{},{:.4},{},{},{},{},{:.6},{},", s.sample_tick, s.timestamp_ns, s.status, s.session, s.session_index, s.completed_laps, s.position, s.session_time_left, s.number_of_laps, s.current_sector_index, s.normalized_car_position, s.is_in_pit, s.is_in_pit_lane, s.mandatory_pit_done, s.missing_mandatory_pits, s.penalty_time, s.penalty_type)?;
        for i in 0..33 { write!(sf, "{},", s.track_status[i])?; }
        writeln!(sf, "{:.6},{:.6},{},{},{},{},{},{},{},{},{},{}", s.clock, s.replay_time_multiplier, s.is_valid_lap, s.global_yellow, s.global_yellow1, s.global_yellow2, s.global_yellow3, s.global_white, s.global_green, s.global_chequered, s.global_red, s.gap_ahead_or_tail_value)?;
    }
    sf.flush()?;
    println!("wrote {}", session_path.display());

    // timing.csv
    let timing_path = out_dir.join("timing.csv");
    let mut tf = BufWriter::new(File::create(&timing_path)?);
    write!(tf, "tick,ns,iCurTime,iLastTime,iBestTime,iSplit,lastSecTime,iDeltaLapTime,isDeltaPos,iEstLapTime,fuelEstLaps,fuelXLap,usedFuel,distTraveled,")?;
    for i in 0..15 { write!(tf, "curTimeStr_{i},")?; }
    for i in 0..15 { write!(tf, "lastTimeStr_{i},")?; }
    for i in 0..15 { write!(tf, "bestTimeStr_{i},")?; }
    for i in 0..15 { write!(tf, "splitStr_{i},")?; }
    for i in 0..15 { write!(tf, "deltaStr_{i},")?; }
    for i in 0..14 { write!(tf, "estStr_{i},")?; }
    writeln!(tf, "estStr_14,obsSlotBeforeISplit")?;
    for t in reader.read_all_timing().unwrap_or_default() {
        write!(tf, "{},{},{},{},{},{},{},{},{},{},{},{},{:.6},{:.6},", t.sample_tick, t.timestamp_ns, t.i_current_time, t.i_last_time, t.i_best_time, t.i_split, t.last_sector_time, t.i_delta_lap_time, t.is_delta_positive, t.i_estimated_lap_time, t.fuel_estimated_laps.to_bits(), t.fuel_x_lap, t.used_fuel, t.distance_traveled)?;
        for i in 0..15 { write!(tf, "{},", t.current_time_str[i])?; }
        for i in 0..15 { write!(tf, "{},", t.last_time_str[i])?; }
        for i in 0..15 { write!(tf, "{},", t.best_time_str[i])?; }
        for i in 0..15 { write!(tf, "{},", t.split_str[i])?; }
        for i in 0..15 { write!(tf, "{},", t.delta_lap_time_str[i])?; }
        for i in 0..15 { write!(tf, "{},", t.estimated_lap_time_str[i])?; }
        writeln!(tf, "{}", t.observed_slot_before_i_split)?;
    }
    tf.flush()?;
    println!("wrote {}", timing_path.display());

    // controls.csv
    let ctrl_path = out_dir.join("controls.csv");
    let mut cf = BufWriter::new(File::create(&ctrl_path)?);
    writeln!(cf, "tick,ns,physId,gfxId,speed,gas,brake,clutch,steer,gear,rpms,fuel")?;
    for c in reader.read_all_controls().unwrap_or_default() {
        writeln!(cf, "{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{:.6}", c.sample_tick, c.timestamp_ns, c.physics_packet_id, c.graphics_packet_id, c.speed_kmh, c.gas, c.brake, c.clutch, c.steer_angle, c.gear, c.rpms, c.fuel)?;
    }
    cf.flush()?;
    println!("wrote {}", ctrl_path.display());

    println!("restore complete: {} files in {}", 4, out_dir.display());
    Ok(())
}

fn import_raw_command(args: &[String]) -> TelemetryResult<()> {
    let in_dir = required_path(args, "--in-dir")?;
    let out = required_path(args, "--out")?;

    // Read metadata
    let meta_path = in_dir.join("metadata.txt");
    let mut track = String::from("unknown");
    let mut car = String::from("unknown_car");
    let mut poll_hz = 120.0f64;
    let mut chunk_rows = 256usize;
    for line in BufReader::new(File::open(&meta_path)?).lines() {
        let line = line?;
        if let Some((k, v)) = line.split_once('=') {
            match k {
                "track" => track = v.to_string(),
                "car" => car = v.to_string(),
                "poll_hz" => { if let Ok(f) = v.parse() { poll_hz = f; } }
                "chunk_rows" => { if let Ok(n) = v.parse() { chunk_rows = n; } }
                _ => {}
            }
        }
    }

    // Read session CSV → build session samples
    let session_path = in_dir.join("session.csv");
    let _session_header = BufReader::new(File::open(&session_path)?).lines().next().unwrap_or(Ok(String::new()))?;
    let mut session_samples: Vec<SessionSample> = Vec::new();
    for line in BufReader::new(File::open(&session_path)?).lines().skip(1) {
        let line = line?;
        let vals: Vec<&str> = line.split(',').collect();
        if vals.len() < 62 { continue; }
        let get_i32 = |i: usize| vals[i].parse::<i32>().unwrap_or(0);
        let get_f32 = |i: usize| vals[i].parse::<f32>().unwrap_or(0.0);
        let get_u64 = |i: usize| vals[i].parse::<u64>().unwrap_or(0);
        let mut ts = [0u16; 33];
        for i in 0..33 { ts[i] = vals[16 + i].parse().unwrap_or(0); }
        session_samples.push(SessionSample {
            sample_tick: get_u64(0), timestamp_ns: get_u64(1),
            status: get_i32(2), session: get_i32(3), session_index: get_i32(4),
            completed_laps: get_i32(5), position: get_i32(6),
            session_time_left: get_f32(7), number_of_laps: get_i32(8),
            current_sector_index: get_i32(9), normalized_car_position: get_f32(10),
            is_in_pit: get_i32(11), is_in_pit_lane: get_i32(12),
            mandatory_pit_done: get_i32(13), missing_mandatory_pits: get_i32(14),
            penalty_time: get_f32(15), penalty_type: get_i32(16),
            track_status: ts, clock: get_f32(49), replay_time_multiplier: get_f32(50),
            is_valid_lap: get_i32(51), global_yellow: get_i32(52),
            global_yellow1: get_i32(53), global_yellow2: get_i32(54), global_yellow3: get_i32(55),
            global_white: get_i32(56), global_green: get_i32(57),
            global_chequered: get_i32(58), global_red: get_i32(59),
            gap_ahead_or_tail_value: get_i32(60),
        });
    }

    // Read timing CSV → build timing samples, keyed by tick
    let timing_path = in_dir.join("timing.csv");
    let mut timing_by_tick: HashMap<u64, TimingSample> = HashMap::new();
    if timing_path.exists() {
        for line in BufReader::new(File::open(&timing_path)?).lines().skip(1) {
            let line = line?;
            let vals: Vec<&str> = line.split(',').collect();
            if vals.len() < 105 { continue; }
            let get_i32 = |i: usize| vals[i].parse::<i32>().unwrap_or(0);
            let get_f32 = |i: usize| vals[i].parse::<f32>().unwrap_or(0.0);
            let get_u64 = |i: usize| vals[i].parse::<u64>().unwrap_or(0);
            let get_u16_15 = |start: usize| -> [u16; 15] {
                let mut a = [0u16; 15];
                for i in 0..15 { a[i] = vals[start + i].parse().unwrap_or(0); }
                a
            };
            let get_f32_bits = |i: usize| {
                let bits: u32 = vals[i].parse().unwrap_or(0);
                f32::from_bits(bits)
            };
            let t = TimingSample {
                sample_tick: get_u64(0), timestamp_ns: get_u64(1),
                i_current_time: get_i32(2), i_last_time: get_i32(3), i_best_time: get_i32(4),
                i_split: get_i32(5), last_sector_time: get_i32(6),
                i_delta_lap_time: get_i32(7), is_delta_positive: get_i32(8),
                i_estimated_lap_time: get_i32(9),
                fuel_estimated_laps: get_f32_bits(10), fuel_x_lap: get_f32(11),
                used_fuel: get_f32(12), distance_traveled: get_f32(13),
                current_time_str: get_u16_15(14), last_time_str: get_u16_15(29),
                best_time_str: get_u16_15(44), split_str: get_u16_15(59),
                delta_lap_time_str: get_u16_15(74), estimated_lap_time_str: get_u16_15(89),
                observed_slot_before_i_split: get_i32(104),
            };
            timing_by_tick.insert(t.sample_tick, t);
        }
    }

    // --- Fix 4-byte struct shift for ALL session columns ---
    // OLD code (missing Penalty field) caused all fields after penalty_type
    // to read from ACC memory positions shifted 4 bytes left. Each session
    // column's true value is in the NEXT column's position in the struct.
    //
    // Recoverable remappings:
    //   is_in_pit_lane      ← mandatory_pit_done  (ACC is_in_pit_lane)
    //   is_valid_lap        ← Timing fuel_estimated_laps (ACC is_valid_lap)
    //   global_yellow       ← global_yellow1       (ACC global_yellow)
    //   global_yellow1      ← global_yellow2       (ACC global_yellow1)
    //   global_yellow2      ← global_yellow3       (ACC global_yellow2)
    //   global_yellow3      ← global_white          (ACC global_yellow3)
    //   global_white        ← global_green          (ACC global_white)
    //   global_green        ← global_chequered      (ACC global_green)
    //   global_chequered    ← global_red            (ACC global_chequered)
    //   global_red          ← gap_ahead             (ACC global_red)
    //
    // Unrecoverable (garbled or not stored):
    //   mandatory_pit_done, missing_mandatory_pits, track_status, clock,
    //   gap_ahead (was ACC strategy_tyre_set at OLD position)
    for s in &mut session_samples {
        // is_valid_lap from Timing fuel_estimated_laps (ACC is_valid_lap position)
        let valid_from_fuel = timing_by_tick.get(&s.sample_tick)
            .map(|t| t.fuel_estimated_laps > 0.0f32)
            .unwrap_or(false);
        s.is_valid_lap = valid_from_fuel as i32;

        // is_in_pit_lane ← mandatory_pit_done (shifted by 4B → reads ACC is_in_pit_lane)
        s.is_in_pit_lane = s.mandatory_pit_done;

        // global flag chain: each reads the previous field's ACC position
        // global_yellow   ← global_yellow1    (ACC global_yellow)
        // global_yellow1  ← global_yellow2    (ACC global_yellow1)
        // global_yellow2  ← global_yellow3    (ACC global_yellow2)
        // global_yellow3  ← global_white      (ACC global_yellow3)
        // global_white    ← global_green      (ACC global_white)
        // global_green    ← global_chequered  (ACC global_green)
        // global_chequered← global_red        (ACC global_chequered)
        // global_red      ← gap_ahead         (ACC global_red)
        let saved_gap = s.gap_ahead_or_tail_value;
        let saved_red = s.global_red;
        let saved_cheq = s.global_chequered;
        let saved_green = s.global_green;
        let saved_white = s.global_white;
        let saved_y3 = s.global_yellow3;
        let saved_y2 = s.global_yellow2;
        let saved_y1 = s.global_yellow1;

        s.global_red = saved_gap;           // was ACC global_red
        s.global_chequered = saved_red;     // was ACC global_chequered
        s.global_green = saved_cheq;        // was ACC global_green
        s.global_white = saved_green;       // was ACC global_white
        s.global_yellow3 = saved_white;     // was ACC global_yellow3
        s.global_yellow2 = saved_y3;        // was ACC global_yellow2
        s.global_yellow1 = saved_y2;        // was ACC global_yellow1
        s.global_yellow = saved_y1;         // was ACC global_yellow
    }

    // Build frames and write new acctlm
    let metadata = SessionMetadata::new(track, car, poll_hz);
    let config = LiveTelemetryConfig { poll_hz, chunk_rows };
    let mut writer = BinaryTelemetryWriter::create_file(&out, metadata, config)?;

    for s in &session_samples {
        let mut tm = timing_by_tick.get(&s.sample_tick).cloned().unwrap_or_default();
        tm.sample_tick = s.sample_tick;
        tm.timestamp_ns = s.timestamp_ns;
        writer.write_frame(TelemetryFrame {
            sample_tick: s.sample_tick,
            timestamp_ns: s.timestamp_ns,
            controls: ControlSample::default(),
            motion: Default::default(),
            tyres: Default::default(),
            powertrain: Default::default(),
            session: s.clone(),
            timing: tm,
            car_state: Default::default(),
            environment: Default::default(),
            other_cars: Default::default(),
        })?;
    }

    let (_file, summary) = writer.finish()?;
    println!("import-raw complete: {} samples, {} chunks -> {}", summary.total_samples, summary.chunk_count, out.display());
    Ok(())
}

fn print_usage() {
    println!(
        "ACC live telemetry\n\n\
commands:\n  \
generate-mock --out <file> [--samples 1000] [--poll-hz 120] [--chunk-rows 256]\n  \
record [--out <file> | --out-dir <dir>] [--poll-hz 120] [--chunk-rows 256] [--status-interval-ms 2000] [--flush-interval-ms 2000]\n  \
inspect --input <file>\n  \
export --input <file> [--out <file>] [--format csv]\n  \
laps --input <file>\n  \
restore --input <file> [--out-dir <dir>]\n  \
import-raw --in-dir <dir> --out <file>\n  \
help"
    )
}