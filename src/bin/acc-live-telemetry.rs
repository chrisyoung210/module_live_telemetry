use module_live_telemetry::{
    parse_raw_frame, shmem::AccGameStatus, shmem::AccSharedMemoryReader,
    BinaryTelemetryReader, BinaryTelemetryWriter, ControlSample,
    LapIndexEntry, LiveTelemetryConfig, SessionMetadata, SPageFileStatic,
    TelemetryError, TelemetryResult, TimingSample,
};
use std::env;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};
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
        "record" => record_command(&args),
        "record-raw" => record_raw_command(&args),
        "inspect" => inspect_command(&args),
        "export" => export_command(&args),
        "laps" => laps_command(&args),
        "session-info" => session_info_command(&args),
        "parse-raw" => parse_raw_command(&args),
        "build-lap-index" => build_lap_index_command(&args),
        "export-lap-field" => export_lap_field_command(&args),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => Err(TelemetryError::InvalidArgument(format!(
            "unknown command '{command}'"
        ))),
    }
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
                    println!("shared memory disappeared; finished recording {} samples in {} chunks",
                        summary.total_samples, summary.chunk_count);
                    if let Some(ref p) = output_path { append_lap_index_to_file(p); }
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
                let mut metadata = SessionMetadata::new(
                    session.track_name,
                    session.car_model,
                    poll_hz,
                );
                // Read full static page once per session
                if let Ok(static_bytes) = active_reader.read_static_bytes() {
                    // Parse static page for individual metadata fields
                    let stat = SPageFileStatic::from_raw(&static_bytes);
                    metadata.sm_version = stat.sm_version_str();
                    metadata.ac_version = stat.ac_version_str();
                    metadata.number_of_sessions = stat.number_of_sessions;
                    metadata.num_cars = stat.num_cars;
                    metadata.sector_count = stat.sector_count;
                    metadata.max_rpm = stat.max_rpm;
                    metadata.max_torque = stat.max_torque;
                    metadata.max_power = stat.max_power;
                    metadata.max_fuel = stat.max_fuel;
                    metadata.penalties_enabled = stat.penalties_enabled;
                    metadata.raw_static_bytes = static_bytes;
                }
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
                            append_lap_index_to_file(path);
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
                append_lap_index_to_file(path);
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
        // Use `normalized_car_position` (0.0鈥?.0, wraps at S/F line).
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
            // With fixed struct, is_valid_lap is 0/1 flag throughout.
            // normPos update lags 2-3 ticks behind iCurTime reset, so check
            // the last 3 samples of the lap instead of just the last one.
            let end = cross_idx.saturating_sub(1);
            let start = end.saturating_sub(2).max(lap_start_idx);
            let valid = !is_out_lap
                && !session_samples[start..=end].iter().any(|s| s.is_valid_lap == 0);

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
            let end_3 = last_idx.saturating_sub(2).max(lap_start_idx);
            let valid = !is_out_lap
                && !session_samples[end_3..=last_idx].iter().any(|s| s.is_valid_lap == 0);

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

fn record_raw_command(args: &[String]) -> TelemetryResult<()> {
    let out = required_path(args, "--out")?;
    let poll_hz = optional_f64(args, "--poll-hz", 120.0)?;
    let status_interval_ms = optional_u64(args, "--status-interval-ms", 2000)?;
    let poll_interval = Duration::from_secs_f64(1.0 / poll_hz.max(1.0));
    let status_interval = Duration::from_millis(status_interval_ms.max(250));

    ensure_parent_dir(&out)?;

    println!("waiting for ACC shared memory...");
    let mut reader: Option<AccSharedMemoryReader> = None;
    let mut raw_file: Option<std::fs::File> = None;
    let mut sample_tick = 0u64;
    let mut recording_started_at = Instant::now();
    let mut last_status_log = Instant::now() - status_interval;

    loop {
        if reader.is_none() {
            match AccSharedMemoryReader::open() {
                Ok(opened) => {
                    println!("connected to ACC shared memory");
                    reader = Some(opened);
                }
                Err(err) => {
                    if last_status_log.elapsed() >= status_interval {
                        println!("ACC not available yet: {err}");
                        last_status_log = Instant::now();
                    }
                    sleep_remaining(Instant::now(), poll_interval);
                    continue;
                }
            }
        }

        let active_reader = reader.as_mut().expect("reader exists");
        let status = match active_reader.status() {
            Ok(s) => s,
            Err(err) => {
                if let Some(mut f) = raw_file.take() { let _ = f.flush(); }
                if last_status_log.elapsed() >= status_interval {
                    println!("lost ACC shared memory: {err}");
                    last_status_log = Instant::now();
                }
                reader = None;
                sleep_remaining(Instant::now(), poll_interval);
                continue;
            }
        };

        if !status.is_live() {
            if last_status_log.elapsed() >= status_interval {
                println!("ACC status: {} {}",
                    status.label(),
                    if raw_file.is_none() { "(waiting for live session...)" } else { "" });
                last_status_log = Instant::now();
            }
            if raw_file.is_some() && !status.is_pause() {
                if let Some(mut f) = raw_file.take() { f.flush()?; }
                println!("session ended; raw recording saved to {}", out.display());
                return Ok(());
            }
            sleep_remaining(Instant::now(), poll_interval);
            continue;
        }

        let tick_start = Instant::now();
        if raw_file.is_none() {
            let phys = active_reader.read_raw_physics()?;
            let graph = active_reader.read_raw_graphics()?;
            let stat = active_reader.read_raw_static()?;
            let mut f = std::fs::File::create(&out)?;
            f.write_all(b"ACCMR\0")?;
            f.write_all(&1u16.to_le_bytes())?;
            f.write_all(&poll_hz.to_le_bytes())?;
            f.write_all(&(phys.len() as u32).to_le_bytes())?;
            f.write_all(&(graph.len() as u32).to_le_bytes())?;
            f.write_all(&(stat.len() as u32).to_le_bytes())?;
            f.write_all(&stat)?;
            raw_file = Some(f);
            recording_started_at = Instant::now();
            sample_tick = 0;
            println!("recording started: {} (static={}B)", out.display(), stat.len());
            continue;
        }

        let ts_ns = recording_started_at.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        let phys = active_reader.read_raw_physics()?;
        let graph = active_reader.read_raw_graphics()?;

        if let Some(ref mut f) = raw_file {
            f.write_all(&sample_tick.to_le_bytes())?;
            f.write_all(&ts_ns.to_le_bytes())?;
            f.write_all(&phys)?;
            f.write_all(&graph)?;
            if last_status_log.elapsed() >= status_interval {
                let frame_sz = 8 + 8 + phys.len() + graph.len();
                println!("recording: {} frames, last tick={sample_tick}", (f.stream_position()? - 28) / frame_sz as u64);
                last_status_log = Instant::now();
            }
        }
        sample_tick = sample_tick.saturating_add(1);
        sleep_remaining(tick_start, poll_interval);
    }
}

fn parse_raw_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let out = required_path(args, "--out")?;
    let poll_hz = optional_f64(args, "--poll-hz", 120.0)?;
    let chunk_rows = optional_usize(args, "--chunk-rows", 256)?;

    let mut file = std::fs::File::open(&input)?;
    let mut magic = [0u8; 6];
    file.read_exact(&mut magic)?;
    if &magic != b"ACCMR\0" {
        return Err(TelemetryError::InvalidFormat("bad ACCMR magic".into()));
    }
    let mut ver_buf = [0u8; 2];
    file.read_exact(&mut ver_buf)?;
    let _version = u16::from_le_bytes(ver_buf);
    let mut hz_buf = [0u8; 8];
    file.read_exact(&mut hz_buf)?;
    let file_poll_hz = f64::from_le_bytes(hz_buf);
    let mut sz_buf = [0u8; 4];
    file.read_exact(&mut sz_buf)?; let phys_sz = u32::from_le_bytes(sz_buf) as usize;
    file.read_exact(&mut sz_buf)?; let graph_sz = u32::from_le_bytes(sz_buf) as usize;
    file.read_exact(&mut sz_buf)?; let stat_sz = u32::from_le_bytes(sz_buf) as usize;
    let use_hz = if poll_hz > 0.0 { poll_hz } else { file_poll_hz };

    let mut stat_bytes = vec![0u8; stat_sz];
    file.read_exact(&mut stat_bytes)?;
    let car = String::from_utf16_lossy(&stat_bytes[68..134].chunks_exact(2).map(|b| u16::from_le_bytes([b[0],b[1]])).collect::<Vec<u16>>()).trim_end_matches('\0').to_string();
    let track = String::from_utf16_lossy(&stat_bytes[134..200].chunks_exact(2).map(|b| u16::from_le_bytes([b[0],b[1]])).collect::<Vec<u16>>()).trim_end_matches('\0').to_string();
    let track = if track.is_empty() { "unknown".into() } else { track };
    let car = if car.is_empty() { "unknown_car".into() } else { car };

    let frame_size = 8 + 8 + phys_sz + graph_sz;
    let mut metadata = SessionMetadata::new(&track, &car, use_hz);
    // Store full raw static page for later parsing
    metadata.raw_static_bytes = stat_bytes.clone();
    // Parse static page to populate individual metadata fields
    let stat = SPageFileStatic::from_raw(&stat_bytes);
    metadata.sm_version = stat.sm_version_str();
    metadata.ac_version = stat.ac_version_str();
    metadata.number_of_sessions = stat.number_of_sessions;
    metadata.num_cars = stat.num_cars;
    metadata.sector_count = stat.sector_count;
    metadata.max_rpm = stat.max_rpm;
    metadata.max_torque = stat.max_torque;
    metadata.max_power = stat.max_power;
    metadata.max_fuel = stat.max_fuel;
    metadata.penalties_enabled = stat.penalties_enabled;
    let config = LiveTelemetryConfig { poll_hz: use_hz, chunk_rows };
    let mut writer = BinaryTelemetryWriter::create_file(&out, metadata, config)?;

    println!("reading raw frames...");
    let mut buf = vec![0u8; frame_size];
    loop {
        match file.read_exact(&mut buf) {
            Ok(()) => {
                let tick = u64::from_le_bytes(buf[0..8].try_into().unwrap());
                let ns = u64::from_le_bytes(buf[8..16].try_into().unwrap());
                let phys = &buf[16..16+phys_sz];
                let graph = &buf[16+phys_sz..16+phys_sz+graph_sz];
                let frame = parse_raw_frame(tick, ns, phys, graph)?;
                writer.write_frame(frame)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
    }

    let (_f, summary) = writer.finish()?;
    println!("parse-raw: {} samples to {}", summary.total_samples, out.display());
    Ok(())
}

fn build_lap_index_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let reader = BinaryTelemetryReader::open(&input)?;
    let session_samples = reader.read_all_session().unwrap_or_default();
    if session_samples.is_empty() {
        return Err(TelemetryError::InvalidArgument("no session data".into()));
    }

    // Detect lap boundaries using normalized_car_position
    let mut crossings: Vec<usize> = Vec::new();
    for i in 1..session_samples.len() {
        let prev_pos = session_samples[i-1].normalized_car_position;
        let cur_pos = session_samples[i].normalized_car_position;
        if prev_pos > 0.8 && cur_pos < 0.2 { crossings.push(i); }
    }

    let mut entries: Vec<LapIndexEntry> = Vec::new();
    let mut start_idx: usize = 0;
    let mut lap_number: i32 = 0;

    for &cross_idx in &crossings {
        let sample_count = (cross_idx - start_idx) as u32;
entries.push(LapIndexEntry {
            lap_number, start_tick: session_samples[start_idx].sample_tick,
            end_tick: session_samples[cross_idx-1].sample_tick,
            sample_count, is_valid: (lap_number != 0 && session_samples[cross_idx-1].is_valid_lap != 0) as i32,
            is_out_lap: (lap_number == 0) as i32,
        });
        start_idx = cross_idx;
        lap_number += 1;
    }
    // Last (incomplete) lap
    let last_idx = session_samples.len() - 1;
    entries.push(module_live_telemetry::types::LapIndexEntry {
        lap_number, start_tick: session_samples[start_idx].sample_tick,
        end_tick: session_samples[last_idx].sample_tick,
        sample_count: (last_idx - start_idx + 1) as u32,
        is_valid: (lap_number != 0) as i32,
        is_out_lap: (lap_number == 0) as i32,
    });

    // Append to file
    let mut file = std::fs::OpenOptions::new().append(true).open(&input)?;
    file.write_all(&module_live_telemetry::format::LAP_INDEX_MAGIC)?;
    file.write_all(&(entries.len() as u32).to_le_bytes())?;
    for e in &entries {
        file.write_all(&e.lap_number.to_le_bytes())?;
        file.write_all(&e.start_tick.to_le_bytes())?;
        file.write_all(&e.end_tick.to_le_bytes())?;
        file.write_all(&e.sample_count.to_le_bytes())?;
        file.write_all(&e.is_valid.to_le_bytes())?;
        file.write_all(&e.is_out_lap.to_le_bytes())?;
    }
    println!("lap index: {} laps appended to {}", entries.len(), input.display());
    Ok(())
}

fn append_lap_index_to_file(path: &std::path::Path) {
    // Best-effort: silently skip if file doesn't exist or has no session data
    if let Ok(reader) = BinaryTelemetryReader::open(path) {
        let samples = reader.read_all_session().unwrap_or_default();
        if samples.len() < 2 { return; }
        // Detect lap boundaries
        let mut crossings: Vec<usize> = Vec::new();
        for i in 1..samples.len() {
            let p = samples[i-1].normalized_car_position;
            let c = samples[i].normalized_car_position;
            if p > 0.8 && c < 0.2 { crossings.push(i); }
        }
        let mut entries = Vec::new();
        let mut start = 0usize;
        let mut num = 0i32;
        for &cx in &crossings {
            entries.push(LapIndexEntry { lap_number: num, start_tick: samples[start].sample_tick,
                end_tick: samples[cx-1].sample_tick, sample_count: (cx-start) as u32,
                is_valid: (num != 0 && samples[cx-1].is_valid_lap != 0) as i32,
                is_out_lap: (num == 0) as i32 });
            start = cx; num += 1;
        }
        let last = samples.len() - 1;
        entries.push(LapIndexEntry { lap_number: num, start_tick: samples[start].sample_tick,
            end_tick: samples[last].sample_tick, sample_count: (last-start+1) as u32,
            is_valid: (num != 0) as i32, is_out_lap: (num == 0) as i32 });
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
            let _ = f.write_all(&module_live_telemetry::format::LAP_INDEX_MAGIC);
            let _ = f.write_all(&(entries.len() as u32).to_le_bytes());
            for e in &entries {
                let _ = f.write_all(&e.lap_number.to_le_bytes());
                let _ = f.write_all(&e.start_tick.to_le_bytes());
                let _ = f.write_all(&e.end_tick.to_le_bytes());
                let _ = f.write_all(&e.sample_count.to_le_bytes());
                let _ = f.write_all(&e.is_valid.to_le_bytes());
                let _ = f.write_all(&e.is_out_lap.to_le_bytes());
            }
        }
    }
}

fn export_lap_field_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let lap_num = optional_usize(args, "--lap", 0)?;
    let fields_str = required_string(args, "--fields")?;
    let out = optional_path(args, "--out").unwrap_or_else(|| PathBuf::from("lap_fields.csv"));
    let field_names: Vec<&str> = fields_str.split(',').map(|s| s.trim()).collect();

    // Validate all field names before any file I/O
    let valid_fields: &[&str] = &[
        // Session
        "status", "session", "sessionIndex", "completedLaps", "position",
        "sessionTimeLeft", "numberOfLaps", "currentSectorIndex", "normalizedCarPosition",
        "isInPit", "isInPitLane", "mandatoryPitDone", "missingMandatoryPits",
        "penaltyTime", "penaltyType", "clock", "replayTimeMultiplier", "isValidLap",
        "globalYellow", "globalYellow1", "globalYellow2", "globalYellow3",
        "globalWhite", "globalGreen", "globalChequered", "globalRed",
        "gapAheadOrTailValue", "flag", "gapBehind",
        // Timing
        "iCurrentTime", "iLastTime", "iBestTime", "iSplit", "lastSectorTime",
        "iDeltaLapTime", "isDeltaPositive", "iEstimatedLapTime",
        "fuelEstimatedLaps", "fuelXLap", "usedFuel", "distanceTraveled",
        // Controls
        "speedKmh", "gas", "brake", "clutch", "steerAngle", "gear", "rpms", "fuel",
        "physicsPacketId", "graphicsPacketId",
    ];

    let normalize = |s: &str| -> String {
        s.chars().filter(|c| *c != '_').collect::<String>().to_lowercase()
    };

    let lookup: std::collections::HashMap<String, &str> = valid_fields.iter()
        .map(|&f| (normalize(f), f))
        .collect();

    let mut errors = Vec::new();
    for fname in &field_names {
        if valid_fields.contains(fname) {
            continue;
        }
        let key = normalize(fname);
        if let Some(suggest) = lookup.get(&key) {
            errors.push(format!("unknown field '{}'; did you mean '{}'?", fname, suggest));
        } else {
            errors.push(format!("unknown field '{}'", fname));
        }
    }
    if !errors.is_empty() {
        return Err(TelemetryError::InvalidArgument(errors.join("\n")));
    }

    let reader = BinaryTelemetryReader::open(&input)?;
    let session = reader.read_all_session().unwrap_or_default();
    let timing = reader.read_all_timing().unwrap_or_default();

    // Build timing lookup by tick
    let timing_by_tick: std::collections::HashMap<u64, &TimingSample> = timing.iter().map(|t| (t.sample_tick, t)).collect();
    let controls = reader.read_all_controls().unwrap_or_default();
    let ctrl_by_tick: std::collections::HashMap<u64, &ControlSample> = controls.iter().map(|c| (c.sample_tick, c)).collect();

    // Find lap boundaries
    let lap_entries = reader.lap_index();
    let (start_tick, end_tick) = if !lap_entries.is_empty() && (lap_num as usize) < lap_entries.len() {
        (lap_entries[lap_num].start_tick, lap_entries[lap_num].end_tick)
    } else {
        // Fallback: scan session data
        let mut crossings: Vec<usize> = Vec::new();
        for i in 1..session.len() {
            if session[i-1].normalized_car_position > 0.8 && session[i].normalized_car_position < 0.2 {
                crossings.push(i);
            }
        }
        if lap_num >= crossings.len() + 1 { return Err(TelemetryError::InvalidArgument(format!("lap {} not found", lap_num))); }
        let start_idx = if lap_num == 0 { 0 } else { crossings[lap_num - 1] };
        let end_idx = if lap_num < crossings.len() { crossings[lap_num] - 1 } else { session.len() - 1 };
        (session[start_idx].sample_tick, session[end_idx].sample_tick)
    };

    // Field extractor map
    let mut out_file = BufWriter::new(File::create(&out)?);
    write!(out_file, "tick,ns")?;
    for f in &field_names { write!(out_file, ",{}", f)?; }
    writeln!(out_file)?;

    for s in session.iter().filter(|s| s.sample_tick >= start_tick && s.sample_tick <= end_tick) {
        let t = timing_by_tick.get(&s.sample_tick);
        write!(out_file, "{},{}", s.sample_tick, s.timestamp_ns)?;
        for fname in &field_names {
            let val: String = match *fname {
                // Session fields
                "status" => s.status.to_string(),
                "session" => s.session.to_string(),
                "sessionIndex" => s.session_index.to_string(),
                "completedLaps" => s.completed_laps.to_string(),
                "position" => s.position.to_string(),
                "sessionTimeLeft" => format!("{:.6}", s.session_time_left),
                "numberOfLaps" => s.number_of_laps.to_string(),
                "currentSectorIndex" => s.current_sector_index.to_string(),
                "normalizedCarPosition" => format!("{:.6}", s.normalized_car_position),
                "isInPit" => s.is_in_pit.to_string(),
                "isInPitLane" => s.is_in_pit_lane.to_string(),
                "mandatoryPitDone" => s.mandatory_pit_done.to_string(),
                "missingMandatoryPits" => s.missing_mandatory_pits.to_string(),
                "penaltyTime" => format!("{:.6}", s.penalty_time),
                "penaltyType" => s.penalty_type.to_string(),
                "clock" => format!("{:.6}", s.clock),
                "replayTimeMultiplier" => format!("{:.6}", s.replay_time_multiplier),
                "isValidLap" => s.is_valid_lap.to_string(),
                "globalYellow" => s.global_yellow.to_string(),
                "globalYellow1" => s.global_yellow1.to_string(),
                "globalYellow2" => s.global_yellow2.to_string(),
                "globalYellow3" => s.global_yellow3.to_string(),
                "globalWhite" => s.global_white.to_string(),
                "globalGreen" => s.global_green.to_string(),
                "globalChequered" => s.global_chequered.to_string(),
                "globalRed" => s.global_red.to_string(),
                "gapAheadOrTailValue" => s.gap_ahead_or_tail_value.to_string(),
                "flag" => s.flag.to_string(),
                "gapBehind" => s.gap_behind.to_string(),
                // Timing fields
                "iCurrentTime" => t.map_or("-".into(), |x| x.i_current_time.to_string()),
                "iLastTime" => t.map_or("-".into(), |x| x.i_last_time.to_string()),
                "iBestTime" => t.map_or("-".into(), |x| x.i_best_time.to_string()),
                "iSplit" => t.map_or("-".into(), |x| x.i_split.to_string()),
                "lastSectorTime" => t.map_or("-".into(), |x| x.last_sector_time.to_string()),
                "iDeltaLapTime" => t.map_or("-".into(), |x| x.i_delta_lap_time.to_string()),
                "isDeltaPositive" => t.map_or("-".into(), |x| x.is_delta_positive.to_string()),
                "iEstimatedLapTime" => t.map_or("-".into(), |x| x.i_estimated_lap_time.to_string()),
                "fuelEstimatedLaps" => t.map_or("-".into(), |x| format!("{:.6}", x.fuel_estimated_laps)),
                "fuelXLap" => t.map_or("-".into(), |x| format!("{:.6}", x.fuel_x_lap)),
                "usedFuel" => t.map_or("-".into(), |x| format!("{:.6}", x.used_fuel)),
                "distanceTraveled" => t.map_or("-".into(), |x| format!("{:.6}", x.distance_traveled)),
                // Controls fields
                _ => {
                    if let Some(c) = ctrl_by_tick.get(&s.sample_tick) {
                        match *fname {
                            "speedKmh" => format!("{:.6}", c.speed_kmh),
                            "gas" => format!("{:.6}", c.gas),
                            "brake" => format!("{:.6}", c.brake),
                            "clutch" => format!("{:.6}", c.clutch),
                            "steerAngle" => format!("{:.6}", c.steer_angle),
                            "gear" => c.gear.to_string(),
                            "rpms" => c.rpms.to_string(),
                            "fuel" => format!("{:.6}", c.fuel),
                            "physicsPacketId" => c.physics_packet_id.to_string(),
                            "graphicsPacketId" => c.graphics_packet_id.to_string(),
                            _ => "?".into(),
                        }
                    } else { "?".into() }
                },
            };
            write!(out_file, ",{}", val)?;
        }
        writeln!(out_file)?;
    }
    out_file.flush()?;
    println!("exported lap {} ({}-{} ticks) to {}", lap_num, start_tick, end_tick, out.display());
    Ok(())
}

fn required_string<'a>(args: &'a [String], flag: &str) -> TelemetryResult<&'a str> {
    args.iter().position(|a| a == flag)
        .and_then(|i| args.get(i+1))
        .map(|s| s.as_str())
        .ok_or_else(|| TelemetryError::InvalidArgument(format!("missing {flag} <value>")))
}

fn session_info_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let reader = BinaryTelemetryReader::open(&input)?;
    let meta = reader.metadata();
    let summary = reader.summary();

    // Parse static page if available
    let stat = if !meta.raw_static_bytes.is_empty() {
        Some(SPageFileStatic::from_raw(&meta.raw_static_bytes))
    } else {
        None
    };

    println!("=== File ===");
    println!("file: {}", input.display());
    println!("format: ACTL v{}", reader.header().version);
    println!("track: {}", meta.track_name);
    println!("car: {}", meta.car_model);
    println!("poll_hz: {:.3}", meta.poll_hz);
    println!("chunks: {}", summary.chunk_count);
    println!("samples: {}", summary.total_samples);

    // Game version
    if let Some(ref s) = stat {
        println!("sm_version: {}", s.sm_version_str());
        println!("ac_version: {}", s.ac_version_str());
    }

    // Vehicle info from static page
    if let Some(ref s) = stat {
        println!("\n=== Vehicle ===");
        println!("max_rpm: {}", s.max_rpm);
        println!("max_power: {:.1} kW", s.max_power);
        println!("max_torque: {:.1} Nm", s.max_torque);
        println!("max_fuel: {:.1} L", s.max_fuel);
        println!("max_turbo_boost: {:.1}", s.max_turbo_boost);
        println!("suspension_max_travel: {:?}", s.suspension_max_travel);
        println!("tyre_radius: {:?}", s.tyre_radius);
        println!("has_drs: {}", if s.has_drs != 0 { "yes" } else { "no" });
        println!("has_ers: {}", if s.has_ers != 0 { "yes" } else { "no" });
        println!("has_kers: {}", if s.has_kers != 0 { "yes" } else { "no" });
        println!("kers_max_j: {:.0}", s.kers_max_j);
        println!("ers_max_j: {:.0}", s.ers_max_j);
        println!("engine_brake_settings_count: {}", s.engine_brake_settings_count);
        println!("ers_power_controller_count: {}", s.ers_power_controller_count);
    }

    // Track info
    if let Some(ref s) = stat {
        println!("\n=== Track ===");
        println!("sector_count: {}", s.sector_count);
        println!("track_configuration: {}", utf16_trim(&s.track_configuration));
        println!("track_spline_length: {:.1}", s.track_spline_length);
        println!("is_timed_race: {}", if s.is_timed_race != 0 { "yes" } else { "no" });
        println!("has_extra_lap: {}", if s.has_extra_lap != 0 { "yes" } else { "no" });
        println!("reversed_grid_positions: {}", s.reversed_grid_positions);
        println!("pit_window_start: {}", s.pit_window_start);
        println!("pit_window_end: {}", s.pit_window_end);
    }

    // Penalties & aids
    if let Some(ref s) = stat {
        println!("\n=== Rules & Aids ===");
        println!("penalties_enabled: {}", if s.penalties_enabled != 0 { "yes" } else { "no" });
        println!("aid_fuel_rate: {:.1}", s.aid_fuel_rate);
        println!("aid_tire_rate: {:.1}", s.aid_tire_rate);
        println!("aid_mechanical_damage: {:.1}", s.aid_mechanical_damage);
        println!("aid_allow_tyre_blankets: {}", if s.aid_allow_tyre_blankets != 0 { "yes" } else { "no" });
        println!("aid_stability: {:.1}", s.aid_stability);
        println!("aid_auto_clutch: {}", if s.aid_auto_clutch != 0 { "yes" } else { "no" });
        println!("aid_auto_blip: {}", if s.aid_auto_blip != 0 { "yes" } else { "no" });
    }

    // Tyres
    if let Some(ref s) = stat {
        println!("\n=== Tyres ===");
        println!("dry_tyres_name: {}", utf16_trim(&s.dry_tyres_name));
        println!("wet_tyres_name: {}", utf16_trim(&s.wet_tyres_name));
    }

    // First session sample for runtime info
    let session_samples = reader.read_all_session().unwrap_or_default();
    if let Some(first) = session_samples.first() {
        let session_label = match first.session {
            0 => "PRACTICE",
            1 => "QUALIFY",
            2 => "RACE",
            3 => "HOTLAP",
            4 => "TIME_ATTACK",
            5 => "DRIFT",
            6 => "DRAG",
            7 => "HOTSTINT",
            8 => "HOTLAP_SUPERPOLE",
            _ => "UNKNOWN",
        };
        let status_label = match first.status {
            0 => "OFF",
            1 => "REPLAY",
            2 => "LIVE",
            3 => "PAUSE",
            _ => "UNKNOWN",
        };
        println!("\n=== Session ===");
        println!("session_type: {} ({})", session_label, first.session);
        println!("status: {} ({})", status_label, first.status);
        println!("session_index: {}", first.session_index);
        println!("number_of_laps: {}", first.number_of_laps);
        println!("clock: {:.3}s", first.clock);
        println!("session_time_left: {:.3}s", first.session_time_left);
    }

    // Environment (first sample)
    let env_samples = reader.read_all_environment().unwrap_or_default();
    if let Some(first) = env_samples.first() {
        println!("\n=== Weather ===");
        println!("air_temp: {:.1} C", first.air_temp);
        println!("road_temp: {:.1} C", first.road_temp);
        println!("air_density: {:.4}", first.air_density);
        println!("wind_speed: {:.1} m/s", first.wind_speed);
        println!("wind_direction: {:.1} deg", first.wind_direction);
        println!("surface_grip: {:.2}", first.surface_grip);
        println!("rain_intensity: {}", first.rain_intensity);
        println!("rain_intensity_10min: {}", first.rain_intensity_in_10min);
        println!("rain_intensity_30min: {}", first.rain_intensity_in_30min);
    }

    Ok(())
}

fn utf16_trim(arr: &[u16]) -> String {
    String::from_utf16_lossy(
        &arr.iter()
            .take_while(|&&c| c != 0)
            .copied()
            .collect::<Vec<u16>>(),
    )
}

fn print_usage() {
    println!(
        "ACC live telemetry\n\n\
commands:\n  \
record [--out <file> | --out-dir <dir>] [--poll-hz 120] [--chunk-rows 256]\n  \
record-raw --out <file> [--poll-hz 120] [--status-interval-ms 2000]\n  \
inspect --input <file>\n  \
export --input <file> [--out <file>] [--format csv]\n  \
laps --input <file>\n  \
session-info --input <file>\n  \
parse-raw --input <file> --out <file> [--poll-hz 120] [--chunk-rows 256]\n  \
build-lap-index --input <file>\n  \
export-lap-field --input <file> --lap <N> --fields <f1,f2,...> [--out <file>]\n  \
help"
    )
}
