use crate::types::OtherCarsSample;
use crate::TelemetryFrame;

/// Extract player car XZ world coordinates with out-of-bounds protection.
pub fn player_xz(other: &OtherCarsSample) -> Option<(f32, f32)> {
    let idx = other.player_car_id as usize;
    let base = idx * 3;
    if base + 2 < other.car_coordinates.len() {
        Some((other.car_coordinates[base], other.car_coordinates[base + 2]))
    } else {
        None
    }
}

pub fn speed(frame: &TelemetryFrame) -> f32 {
    let v = &frame.motion.velocity;
    (v[0] * v[0] + v[2] * v[2]).sqrt()
}

/// Find index of last S/F wrap (position 0.8+ → 0.2-) in reference frames.
/// Returns 0 if no wrap found.
#[cfg(test)]
pub fn find_sf_start(frames: &[TelemetryFrame]) -> usize {
    frames
        .windows(2)
        .enumerate()
        .filter(|(_, w)| {
            w[0].session.normalized_car_position > 0.7
                && w[1].session.normalized_car_position < 0.3
        })
        .map(|(i, _)| i + 1)
        .next_back()
        .unwrap_or(0)
}

/// Sub-frame S/F crossing time interpolation between two frames.
/// Returns estimated crossing time in ms.
pub fn sf_cross_time(f0: &TelemetryFrame, f1: &TelemetryFrame) -> f64 {
    let t0 = f0.timing.i_current_time as f64;
    let t1 = f1.timing.i_current_time as f64;
    let p0 = f0.session.normalized_car_position as f64;
    let p1 = f1.session.normalized_car_position as f64;
    let denom = p0 + (1.0 - p1);
    if denom <= 0.0 {
        return t0;
    }
    let ratio = (p0 / denom).clamp(0.0, 1.0);
    t0 + ratio * (t1 - t0)
}

/// Filter out low-speed (pit/grid) frames. `min_speed` in m/s.
#[cfg(test)]
pub fn filter_low_speed(frames: &[TelemetryFrame], min_speed: f32) -> Vec<usize> {
    frames
        .iter()
        .enumerate()
        .filter(|(_, f)| speed(f) >= min_speed)
        .map(|(i, _)| i)
        .collect()
}

/// Build cumulative arc-length array from XZ points.
pub fn build_cum_len(xz: &[(f32, f32)]) -> Vec<f64> {
    let mut cum = Vec::with_capacity(xz.len());
    cum.push(0.0);
    for i in 1..xz.len() {
        let dx = xz[i].0 - xz[i - 1].0;
        let dz = xz[i].1 - xz[i - 1].1;
        cum.push(cum[i - 1] + (dx * dx + dz * dz).sqrt() as f64);
    }
    cum
}

/// Resample (cum_len, t, v) to uniform arc-length bins with linear interpolation.
pub fn resample_s_bins(cum_len: &[f64], t: &[f64], v: &[f32], n: usize) -> (Vec<f64>, Vec<f32>) {
    let total_len = *cum_len.last().unwrap_or(&0.0);
    if total_len <= 0.0 || n == 0 {
        return (vec![0.0; n], vec![0.0; n]);
    }
    let step = total_len / n as f64;
    let mut t_bins = vec![0.0; n];
    let mut v_bins = vec![0.0f32; n];

    let mut j = 0usize;
    for bin in 0..n {
        let target = (bin as f64 + 0.5) * step;
        while j + 1 < cum_len.len() && cum_len[j + 1] < target {
            j += 1;
        }
        if j + 1 < cum_len.len() {
            let seg_len = cum_len[j + 1] - cum_len[j];
            let ratio = if seg_len > 1e-6 {
                ((target - cum_len[j]) / seg_len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            t_bins[bin] = t[j] + ratio * (t[j + 1] - t[j]);
            v_bins[bin] = v[j] + ratio as f32 * (v[j + 1] - v[j]);
        } else {
            t_bins[bin] = *t.last().unwrap_or(&0.0);
            v_bins[bin] = *v.last().unwrap_or(&0.0);
        }
    }
    (t_bins, v_bins)
}

/// Resample (cum_len, t) to uniform arc-length bins (t only).
#[allow(clippy::needless_range_loop)]
pub fn resample_t_bins(cum_len: &[f64], t: &[f64], n: usize) -> Vec<f64> {
    let total_len = *cum_len.last().unwrap_or(&0.0);
    if total_len <= 0.0 || n == 0 {
        return vec![0.0; n];
    }
    let step = total_len / n as f64;
    let mut t_bins = vec![0.0; n];

    let mut j = 0usize;
    for bin in 0..n {
        let target = (bin as f64 + 0.5) * step;
        while j + 1 < cum_len.len() && cum_len[j + 1] < target {
            j += 1;
        }
        if j + 1 < cum_len.len() {
            let seg_len = cum_len[j + 1] - cum_len[j];
            let ratio = if seg_len > 1e-6 {
                ((target - cum_len[j]) / seg_len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            t_bins[bin] = t[j] + ratio * (t[j + 1] - t[j]);
        } else {
            t_bins[bin] = *t.last().unwrap_or(&0.0);
        }
    }
    t_bins
}

/// Build inverse table: s(t) for each 1ms of lap time.
#[allow(clippy::needless_range_loop)]
pub fn build_inv_table(cum_len: &[f64], t: &[f64]) -> Vec<f64> {
    if t.is_empty() || t.len() < 2 {
        return Vec::new();
    }
    let t_base = t[0];
    let t_max = t[t.len() - 1] - t_base;
    if t_max <= 0.0 {
        return vec![0.0; 1];
    }
    let n = t_max.ceil() as usize + 1;
    let mut s_at_t = vec![0.0f64; n];

    let mut j = 0usize;
    for bin in 0..n {
        let target = bin as f64;
        while j + 1 < t.len() && (t[j + 1] - t_base) < target {
            j += 1;
        }
        if j + 1 < t.len() {
            let dt = t[j + 1] - t[j];
            let ratio = if dt > 1e-6 {
                ((target - (t[j] - t_base)) / dt).clamp(0.0, 1.0)
            } else {
                0.0
            };
            s_at_t[bin] = cum_len[j] + ratio * (cum_len[j + 1] - cum_len[j]);
        } else {
            s_at_t[bin] = *cum_len.last().unwrap_or(&0.0);
        }
    }
    s_at_t
}

/// Project point (x,z) onto polyline (pts, cum_len). Returns normalized s (0..1 of total_len).
/// Uses local search around `prev_seg` if provided.
#[cfg(test)]
pub fn project_onto_polyline(
    xz: (f32, f32),
    pts: &[(f32, f32)],
    cum_len: &[f64],
    total_len: f64,
    prev_seg: Option<usize>,
) -> f64 {
    if pts.len() < 2 || total_len <= 0.0 {
        return 0.0;
    }

    let (mut best_seg, mut best_ratio, best_dist) = global_project(xz, pts);

    if let Some(prev) = prev_seg {
        let local = local_project(xz, pts, prev);
        if let Some(seg) = local.0 {
            if local.2 < best_dist {
                best_seg = seg;
                best_ratio = local.1;
            }
        }
    }

    let s = cum_len[best_seg] + best_ratio as f64 * (cum_len[best_seg + 1] - cum_len[best_seg]);
    (s / total_len).clamp(0.0, 1.0)
}

#[cfg(test)]
fn global_project(xz: (f32, f32), pts: &[(f32, f32)]) -> (usize, f32, f32) {
    let mut best_seg = 0;
    let mut best_ratio = 0.0f32;
    let mut best_dist = f32::MAX;
    for i in 0..pts.len() - 1 {
        let (d, t) = project_point_to_segment(xz, pts[i], pts[i + 1]);
        if d < best_dist {
            best_dist = d;
            best_seg = i;
            best_ratio = t;
        }
    }
    (best_seg, best_ratio, best_dist)
}

#[cfg(test)]
fn local_project(xz: (f32, f32), pts: &[(f32, f32)], prev: usize) -> (Option<usize>, f32, f32) {
    let mut best_seg = None;
    let mut best_ratio = 0.0f32;
    let mut best_dist = f32::MAX;
    let start = prev.saturating_sub(4);
    let end = (prev + 4).min(pts.len() - 2);
    for i in start..=end {
        let (d, t) = project_point_to_segment(xz, pts[i], pts[i + 1]);
        if d < best_dist {
            best_dist = d;
            best_seg = Some(i);
            best_ratio = t;
        }
    }
    (best_seg, best_ratio, best_dist)
}

#[cfg(test)]
fn project_point_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let dx = b.0 - a.0;
    let dz = b.1 - a.1;
    let len_sq = dx * dx + dz * dz;
    if len_sq < 1e-10 {
        let d = ((p.0 - a.0) * (p.0 - a.0) + (p.1 - a.1) * (p.1 - a.1)).sqrt();
        return (d, 0.0);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dz) / len_sq).clamp(0.0, 1.0);
    let proj_x = a.0 + t * dx;
    let proj_z = a.1 + t * dz;
    let dist = ((p.0 - proj_x) * (p.0 - proj_x) + (p.1 - proj_z) * (p.1 - proj_z)).sqrt();
    (dist, t)
}

/// Median of up to 3 values (ignores NaN).
#[cfg(test)]
pub fn median3(window: &[f64; 3], count: usize) -> Option<f64> {
    if count == 0 {
        return None;
    }
    let mut v = [window[0], window[1], window[2]];
    let slice = &mut v[..count.min(3)];
    slice.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(slice[slice.len() / 2])
}
