use module_live_telemetry::{
    mock::generate_mock_controls, raw_writer::import_raw_to_v2, shmem::AccGameStatus,
    shmem::AccSharedMemoryReader, BinaryTelemetryReader, BinaryTelemetryWriter, ControlSample,
    LiveTelemetryConfig, RawPageTelemetryConfig, RawPageTelemetryWriter, SessionMetadata,
    TelemetryError, TelemetryFrame, TelemetryResult,
};
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
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
        "raw-info" => raw_info_command(),
        "raw-laps" => raw_laps_command(&args),
        "raw-lap-segments" => raw_lap_segments_command(&args),
        "raw-valid-scan" => raw_valid_scan_command(&args),
        "record-auto" => record_auto_command(&args),
        "import-raw" => import_raw_command(&args),
        "inspect" => inspect_command(&args),
        "export" => export_command(&args),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => Err(TelemetryError::InvalidArgument(format!(
            "unknown command '{command}'"
        ))),
    }
}

fn raw_lap_segments_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let reader = BinaryTelemetryReader::open(&input)?;
    let session = reader.segment_raw_session()?;
    let metadata = &session.metadata;
    let duration_s = match (session.start_time_ns, session.end_time_ns) {
        (Some(start), Some(end)) => end.saturating_sub(start) as f64 / 1_000_000_000.0,
        _ => 0.0,
    };

    println!("file: {}", input.display());
    println!("track: {}", metadata.track_name);
    println!("car: {}", metadata.car_model);
    println!(
        "session_type: {}",
        match (session.session_type, session.session_kind) {
            (Some(raw), Some(kind)) => format!("{} ({})", raw, kind.label()),
            (Some(raw), None) => raw.to_string(),
            (None, _) => "unavailable".to_string(),
        }
    );
    println!("samples: {}", session.sample_count);
    println!("duration_s: {:.3}", duration_s);
    println!("laps: {}", session.laps.len());
    println!("session_static_cluster: metadata(track, car, created_unix_ns, poll_hz, chunk_rows)");
    println!("lap_dynamic_clusters: raw_pages");

    for lap in &session.laps {
        println!(
            "lap #{:03} completed_laps={}..{} complete={} valid={} reason={} samples={} sample_range={}..{} ticks={}..{} time_ns={}..{} duration_s={:.3} lap_time_ms={} norm={:.5}..{:.5} distance_m={:.1}..{:.1} clusters=raw_pages",
            lap.lap_index,
            lap.acc_completed_laps_at_start,
            lap.acc_completed_laps_at_end,
            lap.is_complete,
            format_optional_bool(lap.is_valid),
            lap.boundary_reason
                .map(|reason| reason.label().to_string())
                .unwrap_or_else(|| "recording_end".to_string()),
            lap.sample_count,
            lap.start_sample_index,
            lap.end_sample_index,
            lap.start_tick,
            lap.end_tick,
            lap.start_time_ns,
            lap.end_time_ns,
            lap.duration_ns() as f64 / 1_000_000_000.0,
            format_optional_i32(lap.lap_time_ms),
            lap.min_normalized_car_position,
            lap.max_normalized_car_position,
            lap.start_distance_traveled_m,
            lap.end_distance_traveled_m,
        );
    }

    Ok(())
}

fn raw_valid_scan_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let reader = BinaryTelemetryReader::open(&input)?;
    let pages = reader.read_all_raw_graphics_pages()?;
    let samples = reader.read_all_raw_graphics_samples()?;
    if pages.is_empty() || samples.len() != pages.len() {
        return Err(TelemetryError::InvalidFormat(
            "raw graphics pages are missing or inconsistent".to_string(),
        ));
    }

    let mut finish_indices = Vec::new();
    for i in 1..samples.len() {
        let prev = samples[i - 1];
        let cur = samples[i];
        let current_reset = prev.current_lap_time_ms > 10_000 && cur.current_lap_time_ms < 2_000;
        if prev.completed_laps != cur.completed_laps || current_reset {
            finish_indices.push(i - 1);
        }
    }

    println!("file: {}", input.display());
    println!("samples: {}", samples.len());
    println!("lap_finish_candidate_indices: {:?}", finish_indices);
    if finish_indices.len() < 4 {
        println!("not enough lap finish candidates for the expected out + 3 timed laps");
    }

    let page_size = pages[0].page.len();
    println!("graphics_page_bytes: {page_size}");
    println!("candidate i32 offsets where timed-lap finish values match invalid, invalid, valid = 0,0,1:");

    let timed_finish_indices: Vec<usize> = finish_indices
        .iter()
        .copied()
        .filter(|idx| samples[*idx].current_lap_time_ms > 30_000)
        .collect();

    for offset in (0..=page_size.saturating_sub(4)).step_by(4) {
        let values: Vec<i32> = timed_finish_indices
            .iter()
            .map(|idx| i32::from_le_bytes(pages[*idx].page[offset..offset + 4].try_into().unwrap()))
            .collect();
        if values.len() >= 3 && values[0] == 0 && values[1] == 0 && values[2] == 1 {
            let next_values: Vec<i32> = timed_finish_indices
                .iter()
                .map(|idx| {
                    let next = (*idx + 1).min(pages.len() - 1);
                    i32::from_le_bytes(pages[next].page[offset..offset + 4].try_into().unwrap())
                })
                .collect();
            println!(
                "  offset={} finish_values={:?} next_values={:?}",
                offset, values, next_values
            );
        }
    }

    println!("known lap finish details:");
    for idx in timed_finish_indices {
        let sample = samples[idx];
        println!(
            "  idx={} completedLaps={} currentLapTimeMs={} lastLapTimeMs={} bestLapTimeMs={} t={:.3}s",
            idx,
            sample.completed_laps,
            sample.current_lap_time_ms,
            sample.last_lap_time_ms,
            sample.best_lap_time_ms,
            sample.timestamp_ns as f64 / 1_000_000_000.0
        );
    }

    let timed_only: Vec<usize> = finish_indices
        .iter()
        .copied()
        .filter(|idx| samples[*idx].current_lap_time_ms > 100_000)
        .collect();
    println!("offset dump for full timed lap finish samples:");
    for offset in (1320..=1432).step_by(4) {
        let values: Vec<i32> = timed_only
            .iter()
            .map(|idx| i32::from_le_bytes(pages[*idx].page[offset..offset + 4].try_into().unwrap()))
            .collect();
        let next_values: Vec<i32> = timed_only
            .iter()
            .map(|idx| {
                let next = (*idx + 1).min(pages.len() - 1);
                i32::from_le_bytes(pages[next].page[offset..offset + 4].try_into().unwrap())
            })
            .collect();
        println!(
            "  offset={} finish={:?} next={:?}",
            offset, values, next_values
        );
    }
    Ok(())
}

fn raw_laps_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let reader = BinaryTelemetryReader::open(&input)?;
    let samples = reader.read_all_raw_graphics_samples()?;
    if samples.is_empty() {
        return Err(TelemetryError::InvalidFormat(
            "no raw graphics samples found".to_string(),
        ));
    }

    println!("file: {}", input.display());
    println!("samples: {}", samples.len());
    println!(
        "duration_s: {:.3}",
        (samples.last().unwrap().timestamp_ns - samples.first().unwrap().timestamp_ns) as f64
            / 1_000_000_000.0
    );
    println!("transitions:");

    let mut lap_events = Vec::new();
    for i in 1..samples.len() {
        let prev = samples[i - 1];
        let cur = samples[i];
        let current_reset = prev.current_lap_time_ms > 10_000 && cur.current_lap_time_ms < 2_000;
        if prev.completed_laps != cur.completed_laps
            || current_reset
            || prev.last_lap_time_ms != cur.last_lap_time_ms
        {
            println!(
                "  i={} t={:.3}s completedLaps {}->{} current {}->{} last {}->{} best {}->{} valid {}->{} norm {:.5}->{:.5} dist {:.1}->{:.1}",
                i,
                cur.timestamp_ns as f64 / 1_000_000_000.0,
                prev.completed_laps,
                cur.completed_laps,
                prev.current_lap_time_ms,
                cur.current_lap_time_ms,
                prev.last_lap_time_ms,
                cur.last_lap_time_ms,
                prev.best_lap_time_ms,
                cur.best_lap_time_ms,
                prev.is_valid_lap,
                cur.is_valid_lap,
                prev.normalized_car_position,
                cur.normalized_car_position,
                prev.distance_traveled_m,
                cur.distance_traveled_m,
            );
            if prev.completed_laps != cur.completed_laps || current_reset {
                lap_events.push((i, prev, cur));
            }
        }
    }

    println!("lap buckets by completedLaps:");
    let mut start = 0usize;
    while start < samples.len() {
        let lap = samples[start].completed_laps;
        let mut end = start + 1;
        while end < samples.len() && samples[end].completed_laps == lap {
            end += 1;
        }
        let first = samples[start];
        let last = samples[end - 1];
        let duration_s =
            (last.timestamp_ns.saturating_sub(first.timestamp_ns)) as f64 / 1_000_000_000.0;
        let valid_min = samples[start..end]
            .iter()
            .map(|s| s.is_valid_lap)
            .min()
            .unwrap_or(0);
        let valid_max = samples[start..end]
            .iter()
            .map(|s| s.is_valid_lap)
            .max()
            .unwrap_or(0);
        println!(
            "  completedLaps={} samples={} wall={:.3}s currentMs={}..{} lastMs={}..{} valid={}..{} norm={:.5}..{:.5} dist={:.1}..{:.1}",
            lap,
            end - start,
            duration_s,
            samples[start..end].iter().map(|s| s.current_lap_time_ms).min().unwrap_or(0),
            samples[start..end].iter().map(|s| s.current_lap_time_ms).max().unwrap_or(0),
            samples[start..end].iter().map(|s| s.last_lap_time_ms).min().unwrap_or(0),
            samples[start..end].iter().map(|s| s.last_lap_time_ms).max().unwrap_or(0),
            valid_min,
            valid_max,
            samples[start..end].iter().map(|s| s.normalized_car_position).fold(f32::INFINITY, f32::min),
            samples[start..end].iter().map(|s| s.normalized_car_position).fold(f32::NEG_INFINITY, f32::max),
            samples[start..end].iter().map(|s| s.distance_traveled_m).fold(f32::INFINITY, f32::min),
            samples[start..end].iter().map(|s| s.distance_traveled_m).fold(f32::NEG_INFINITY, f32::max),
        );
        start = end;
    }

    println!("lap completion events:");
    for (i, prev, cur) in lap_events {
        println!(
            "  i={} completedLaps {}->{} currentMs {}->{} lastLapTimeMs={} bestLapTimeMs={} validBefore={} validAfter={} normBefore={:.5} normAfter={:.5}",
            i,
            prev.completed_laps,
            cur.completed_laps,
            prev.current_lap_time_ms,
            cur.current_lap_time_ms,
            cur.last_lap_time_ms,
            cur.best_lap_time_ms,
            prev.is_valid_lap,
            cur.is_valid_lap,
            prev.normalized_car_position,
            cur.normalized_car_position,
        );
    }

    Ok(())
}

fn raw_info_command() -> TelemetryResult<()> {
    let (
        physics_bytes,
        graphics_bytes,
        static_bytes,
        physics_fields,
        graphics_fields,
        static_fields,
        total_fields,
        pages,
    ) = AccSharedMemoryReader::raw_page_layout();
    println!("raw telemetry capture layout:");
    println!("pages_per_sample: {pages}");
    println!("physics_page_bytes: {physics_bytes}");
    println!("graphics_page_bytes: {graphics_bytes}");
    println!("static_page_bytes: {static_bytes}");
    println!("physics_flattened_fields: {physics_fields}");
    println!("graphics_flattened_fields: {graphics_fields}");
    println!("static_flattened_fields: {static_fields}");
    println!("total_flattened_raw_telemetry_fields: {total_fields}");
    println!("record_clock_fields: 2 (sampleTick, timestampNs)");
    println!("default_poll_hz: 120");
    println!("sampling_rule: one raw sample per new ACC physics packet while graphics status is LIVE; PAUSE keeps the file open and writes no samples");
    Ok(())
}

fn record_auto_command(args: &[String]) -> TelemetryResult<()> {
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
    let mut writer: Option<RawPageTelemetryWriter<File>> = None;
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
                let metadata = SessionMetadata::new(session.track_name, session.car_model, poll_hz);
                let config = LiveTelemetryConfig {
                    poll_hz,
                    chunk_rows,
                };
                let (physics_page_size, graphics_page_size, static_page_size) =
                    active_reader.raw_page_sizes();
                let (physics_fields, graphics_fields, static_fields, total_fields) =
                    AccSharedMemoryReader::raw_field_counts();
                let config = RawPageTelemetryConfig {
                    poll_hz: config.poll_hz,
                    chunk_rows: config.chunk_rows,
                    physics_page_size,
                    graphics_page_size,
                    static_page_size,
                };
                writer = Some(RawPageTelemetryWriter::create_file(
                    &path, metadata, config,
                )?);
                output_path = Some(path.clone());
                sample_tick = 0;
                recording_started_at = Instant::now();
                last_flush = Instant::now();
                println!("recording started: {}", path.display());
                println!(
                    "raw capture: physics={} fields ({} bytes), graphics={} fields ({} bytes), static={} fields ({} bytes), total={} flattened raw telemetry fields/sample",
                    physics_fields,
                    physics_page_size,
                    graphics_fields,
                    graphics_page_size,
                    static_fields,
                    static_page_size,
                    total_fields
                );
            }

            let timestamp_ns = recording_started_at
                .elapsed()
                .as_nanos()
                .min(u64::MAX as u128) as u64;
            match active_reader.read_raw_page_sample(sample_tick, timestamp_ns) {
                Ok(Some(sample)) => {
                    if let Some(active_writer) = writer.as_mut() {
                        active_writer.write_sample(sample)?;
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
                        if let Some(path) = output_path {
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
            if let Some(path) = output_path {
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

fn import_raw_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let output = required_path(args, "--output")?;
    let chunk_rows = optional_usize(args, "--chunk-rows", 1024)?;
    println!("importing {} -> {} ...", input.display(), output.display());
    let summary = import_raw_to_v2(&input, &output, chunk_rows)?;
    println!(
        "imported {} samples in {} chunks to {} ({} bytes)",
        summary.total_samples,
        summary.chunk_count,
        output.display(),
        summary.total_bytes
    );
    Ok(())
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
    println!("chunk_rows: {}", metadata.chunk_rows);
    println!("samples: {}", summary.total_samples);
    println!("chunks: {}", summary.chunk_count);
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
            "only --format csv is supported in the MVP".to_string(),
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

fn format_optional_i32(value: Option<i32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_optional_bool(value: Option<bool>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

fn print_usage() {
    println!(
        "ACC live telemetry MVP\n\n\
commands:\n  \
generate-mock --out <file> [--samples 1000] [--poll-hz 120] [--chunk-rows 256]\n  \
raw-info\n  \
raw-laps --input <file>\n  \
raw-lap-segments --input <file>\n  \
raw-valid-scan --input <file>\n  \
record-auto [--out <file> | --out-dir <dir>] [--poll-hz 120] [--chunk-rows 256] [--flush-interval-ms 2000]\n  \
import-raw --input <old_file> --output <new_file> [--chunk-rows 1024]\n  \
inspect --input <file>\n  \
export --input <file> [--out <csv>] [--format csv]"
    );
}

fn required_path(args: &[String], name: &str) -> TelemetryResult<PathBuf> {
    optional_path(args, name).ok_or_else(|| {
        TelemetryError::InvalidArgument(format!("missing required argument {name} <path>"))
    })
}

fn optional_path(args: &[String], name: &str) -> Option<PathBuf> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| PathBuf::from(&pair[1]))
}

fn optional_string(args: &[String], name: &str, default: &str) -> String {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .unwrap_or_else(|| default.to_string())
}

fn optional_usize(args: &[String], name: &str, default: usize) -> TelemetryResult<usize> {
    match args.windows(2).find(|pair| pair[0] == name) {
        Some(pair) => pair[1]
            .parse::<usize>()
            .map_err(|_| TelemetryError::InvalidArgument(format!("invalid value for {name}"))),
        None => Ok(default),
    }
}

fn optional_u64(args: &[String], name: &str, default: u64) -> TelemetryResult<u64> {
    match args.windows(2).find(|pair| pair[0] == name) {
        Some(pair) => pair[1]
            .parse::<u64>()
            .map_err(|_| TelemetryError::InvalidArgument(format!("invalid value for {name}"))),
        None => Ok(default),
    }
}

fn optional_f64(args: &[String], name: &str, default: f64) -> TelemetryResult<f64> {
    match args.windows(2).find(|pair| pair[0] == name) {
        Some(pair) => pair[1]
            .parse::<f64>()
            .map_err(|_| TelemetryError::InvalidArgument(format!("invalid value for {name}"))),
        None => Ok(default),
    }
}

fn sleep_remaining(start: Instant, interval: Duration) {
    let elapsed = start.elapsed();
    if elapsed < interval {
        thread::sleep(interval - elapsed);
    }
}

fn ensure_parent_dir(path: &Path) -> TelemetryResult<()> {
    let dir = if path.extension().is_some() {
        path.parent()
    } else {
        Some(path)
    };
    if let Some(dir) = dir {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    Ok(())
}

fn default_recording_name(track_name: &str, car_model: &str) -> String {
    let unix_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!(
        "live_{}_{}_{}.acctlm",
        unix_secs,
        sanitize_file_part(track_name),
        sanitize_file_part(car_model)
    )
}

fn sanitize_file_part(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}
