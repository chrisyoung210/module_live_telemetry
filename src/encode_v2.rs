//! Column-level encoder/decoder for V2 format.
//!
//! Provides per-column encoding and decoding of telemetry values with
//! PLAIN (little-endian raw) and DELTA (differential) codecs, plus
//! CRC32 integrity verification.
//!
//! # Supported value types
//!
//! | Constant       | Value | Element size |
//! |----------------|-------|-------------|
//! | `TYPE_U64`     | 0x01  | 8 bytes     |
//! | `TYPE_I32`     | 0x02  | 4 bytes     |
//! | `TYPE_F32`     | 0x03  | 4 bytes     |
//! | `TYPE_F64`        | 0x04  | 8 bytes     |
//! | `TYPE_BYTES`      | 0x05  | variable    |
//! | `TYPE_BYTES_F32`  | 0x06  | f32 sub-val |
//! | `TYPE_BYTES_U16`  | 0x07  | u16 sub-val |
//! | `TYPE_BYTES_I32`  | 0x08  | i32 sub-val |
//!
//! # Encoded column format
//!
//! ```text
//! [codec: u8] [value_type: u8] [value_count: u32 LE] [payload ...]
//! ```
//!
//! CRC32 is computed over this entire header+payload. The CRC32 value
//! is stored externally (e.g., in `ColumnEntryV2.crc32`) and verified
//! on decode.

use crate::error::{TelemetryError, TelemetryResult};
use crate::format_v2::{TYPE_BYTES, TYPE_BYTES_F32, TYPE_BYTES_I32, TYPE_BYTES_U16, TYPE_F32, TYPE_F64, TYPE_I32, TYPE_U64};

// ---------------------------------------------------------------------------
// Codec constants
// ---------------------------------------------------------------------------

/// Plain little-endian encoding — each value stored as raw bytes.
pub const CODEC_PLAIN: u8 = 0x00;
/// Delta encoding — first value full, subsequent values as difference from previous.
pub const CODEC_DELTA: u8 = 0x01;

// ---------------------------------------------------------------------------
// Header helpers
// ---------------------------------------------------------------------------

/// Total header size: codec(1) + value_type(1) + value_count(4) = 6 bytes.
const HEADER_BYTES: usize = 6;

/// Build the column header bytes and write them into `out`.
fn push_header(out: &mut Vec<u8>, codec: u8, value_type: u8, value_count: u32) {
    out.push(codec);
    out.push(value_type);
    out.extend_from_slice(&value_count.to_le_bytes());
}

/// Read the column header from a byte slice.
/// Returns (codec, value_type, value_count, bytes_consumed).
fn read_header(data: &[u8]) -> TelemetryResult<(u8, u8, u32)> {
    if data.len() < HEADER_BYTES {
        return Err(TelemetryError::InvalidFormat(
            "column data too short for header".to_string(),
        ));
    }
    let codec = data[0];
    let value_type = data[1];
    let value_count = u32::from_le_bytes([data[2], data[3], data[4], data[5]]);
    Ok((codec, value_type, value_count))
}

/// Check if a value_type is any BYTES variant (incl. sub-typed versions).
fn is_bytes_type(vt: u8) -> bool {
    vt == TYPE_BYTES || vt == TYPE_BYTES_F32 || vt == TYPE_BYTES_U16 || vt == TYPE_BYTES_I32
}

// ---------------------------------------------------------------------------
// encode_column
// ---------------------------------------------------------------------------

/// Encode a column of values into a byte buffer.
///
/// # Parameters
/// - `values`: slice of `f64` values. For non-`TYPE_BYTES`, each `f64`
///   is an independent row value. For `TYPE_BYTES`, `values` contains
///   all sub-values concatenated and `sub_value_count` specifies how many
///   sub-values belong to each logical item.
/// - `value_type`: one of `TYPE_U64`, `TYPE_I32`, `TYPE_F32`, `TYPE_F64`,
///   `TYPE_BYTES`.
/// - `codec`: `CODEC_PLAIN` or `CODEC_DELTA`.
/// - `sub_value_count`: for `TYPE_BYTES`, the number of sub-values per
///   logical item (e.g., 3 for velocity). Ignored for other types.
///
/// # Returns
///
/// `(encoded_bytes, crc32, min_value, max_value)` where `encoded_bytes`
/// includes the 6-byte header and the payload, and `crc32` is the CRC32
/// checksum of `encoded_bytes`.
pub fn encode_column(
    values: &[f64],
    value_type: u8,
    codec: u8,
    sub_value_count: u8,
) -> (Vec<u8>, u32, f64, f64) {
    let (min_val, max_val) = compute_min_max(values, value_type);

    // Build payload (the raw value bytes)
    let payload = match codec {
        CODEC_PLAIN => encode_plain_payload(values, value_type, sub_value_count),
        CODEC_DELTA => encode_delta_payload(values, value_type, sub_value_count),
        _ => encode_plain_payload(values, value_type, sub_value_count), // fallback
    };

    // value_count header field
    let value_count: u32 = if is_bytes_type(value_type) && sub_value_count > 0 {
        // Logical items = values.len() / sub_value_count
        // But we store item count as value_count (each item may have multiple sub-values)
        (values.len() / sub_value_count as usize) as u32
    } else {
        values.len() as u32
    };

    // Build full encoded buffer: header + payload
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    push_header(&mut out, codec, value_type, value_count);
    out.extend_from_slice(&payload);

    // CRC32 over the entire encoded buffer
    let crc = crc32fast::hash(&out);

    (out, crc, min_val, max_val)
}

// ---------------------------------------------------------------------------
// Payload encoding – PLAIN
// ---------------------------------------------------------------------------

fn encode_plain_payload(values: &[f64], value_type: u8, sub_value_count: u8) -> Vec<u8> {
    match value_type {
        TYPE_U64 => encode_values_as::<8>(values, |v| (v as u64).to_le_bytes()),
        TYPE_I32 => encode_values_as::<4>(values, |v| (v as i32).to_le_bytes()),
        TYPE_F32 => encode_values_as::<4>(values, |v| (v as f32).to_le_bytes()),
        TYPE_F64 => encode_values_as::<8>(values, |v| v.to_le_bytes()),
        TYPE_BYTES => {
            if sub_value_count == 0 {
                return Vec::new();
            }
            encode_bytes_plain(values, sub_value_count)
        }
        TYPE_BYTES_F32 => {
            if sub_value_count == 0 {
                return Vec::new();
            }
            encode_bytes_plain_typed::<4>(values, sub_value_count, |v| (v as f32).to_le_bytes())
        }
        TYPE_BYTES_U16 => {
            if sub_value_count == 0 {
                return Vec::new();
            }
            encode_bytes_plain_typed::<2>(values, sub_value_count, |v| (v as u16).to_le_bytes())
        }
        TYPE_BYTES_I32 => {
            if sub_value_count == 0 {
                return Vec::new();
            }
            encode_bytes_plain_typed::<4>(values, sub_value_count, |v| (v as i32).to_le_bytes())
        }
        _ => Vec::new(),
    }
}

/// Generic PLAIN encoder for fixed-size types.
/// Each element is encoded as `N` LE bytes.
fn encode_values_as<const N: usize>(
    values: &[f64],
    encode: impl Fn(f64) -> [u8; N],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * N);
    for &v in values {
        out.extend_from_slice(&encode(v));
    }
    out
}

/// PLAIN encoder for TYPE_BYTES.
/// Each logical item: [sub_value_count: u8] [val0 LE] [val1 LE] ...
fn encode_bytes_plain(values: &[f64], sub_value_count: u8) -> Vec<u8> {
    let per_item = 1 + sub_value_count as usize * 8; // 1 byte count + 8 bytes per f64
    let num_items = values.len() / sub_value_count as usize;
    let mut out = Vec::with_capacity(num_items * per_item);
    let chunks = values.chunks_exact(sub_value_count as usize);
    for chunk in chunks {
        out.push(sub_value_count);
        for &v in chunk {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// PLAIN encoder for typed BYTES variants.
/// Each logical item: [sub_value_count: u8] [val0: N bytes LE] [val1: N bytes LE] ...
fn encode_bytes_plain_typed<const N: usize>(
    values: &[f64],
    sub_value_count: u8,
    encode: impl Fn(f64) -> [u8; N],
) -> Vec<u8> {
    let per_item = 1 + sub_value_count as usize * N;
    let num_items = values.len() / sub_value_count as usize;
    let mut out = Vec::with_capacity(num_items * per_item);
    let chunks = values.chunks_exact(sub_value_count as usize);
    for chunk in chunks {
        out.push(sub_value_count);
        for &v in chunk {
            out.extend_from_slice(&encode(v));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Payload encoding – DELTA
// ---------------------------------------------------------------------------

fn encode_delta_payload(values: &[f64], value_type: u8, sub_value_count: u8) -> Vec<u8> {
    if values.is_empty() {
        return Vec::new();
    }

    match value_type {
        TYPE_U64 => encode_delta_u64(values),
        TYPE_I32 => encode_delta_i32(values),
        // Floats don't delta-compress well; fall back to PLAIN.
        TYPE_F32 | TYPE_F64 => encode_plain_payload(values, value_type, sub_value_count),
        TYPE_BYTES => encode_delta_bytes(values, sub_value_count),
        // Typed BYTES variants fall back to PLAIN — deltas need consistent element size
        TYPE_BYTES_F32 | TYPE_BYTES_U16 | TYPE_BYTES_I32 => {
            encode_plain_payload(values, value_type, sub_value_count)
        }
        _ => Vec::new(),
    }
}

/// DELTA encoder for u64.
/// Layout: first value (8 bytes LE), then i64 deltas (8 bytes LE each).
fn encode_delta_u64(values: &[f64]) -> Vec<u8> {
    let first = values[0] as u64;
    let mut out = Vec::with_capacity(8 + (values.len() - 1) * 8);
    out.extend_from_slice(&first.to_le_bytes());
    let mut prev = first as i64;
    for &v in &values[1..] {
        let cur = v as i64;
        let diff = cur.wrapping_sub(prev);
        out.extend_from_slice(&diff.to_le_bytes());
        prev = cur;
    }
    out
}

/// DELTA encoder for i32.
/// Layout: first value (4 bytes LE), then i32 deltas (4 bytes LE each).
fn encode_delta_i32(values: &[f64]) -> Vec<u8> {
    let first = values[0] as i32;
    let mut out = Vec::with_capacity(4 + (values.len() - 1) * 4);
    out.extend_from_slice(&first.to_le_bytes());
    let mut prev = first;
    for &v in &values[1..] {
        let cur = v as i32;
        let diff = cur.wrapping_sub(prev);
        out.extend_from_slice(&diff.to_le_bytes());
        prev = cur;
    }
    out
}

/// DELTA encoder for TYPE_BYTES.
/// Each logical item: [sub_value_count: u8] then first value (8 bytes per sub),
/// then deltas (4 bytes per sub-value).
fn encode_delta_bytes(values: &[f64], sub_value_count: u8) -> Vec<u8> {
    if sub_value_count == 0 || values.is_empty() {
        return Vec::new();
    }

    let sc = sub_value_count as usize;
    let num_items = values.len() / sc;
    let first_item_size = 1 + sc * 8; // 1 + sub_count * 8 bytes first
    let delta_item_size = 1 + sc * 4; // 1 + sub_count * 4 bytes delta
    let mut out = Vec::with_capacity(first_item_size + (num_items - 1) * delta_item_size);

    let chunks: Vec<_> = values.chunks_exact(sc).collect();
    if chunks.is_empty() {
        return Vec::new();
    }

    // First item: full values
    let first = chunks[0];
    out.push(sub_value_count);
    for &v in first {
        out.extend_from_slice(&v.to_le_bytes());
    }

    // Subsequent items: deltas from previous
    for w in chunks.windows(2) {
        let prev = w[0];
        let cur = w[1];
        out.push(sub_value_count);
        for i in 0..sc {
            let diff = (cur[i] - prev[i]) as f32;
            out.extend_from_slice(&diff.to_le_bytes());
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Min/max computation
// ---------------------------------------------------------------------------

fn compute_min_max(values: &[f64], value_type: u8) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }

    if value_type == TYPE_BYTES {
        // For BYTES, min/max over all sub-values
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for &v in values {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
        }
        (min, max)
    } else {
        let mut min = values[0];
        let mut max = values[0];
        for &v in &values[1..] {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
        }
        (min, max)
    }
}

// ---------------------------------------------------------------------------
// decode_column
// ---------------------------------------------------------------------------

/// Decode column data, verifying the CRC32 checksum.
///
/// # Parameters
/// - `data`: the encoded column bytes (including the 6-byte header).
/// - `expected_crc32`: the CRC32 checksum stored in `ColumnEntryV2.crc32`.
///
/// # Returns
///
/// `Vec<f64>` — all decoded values, flattened. For `TYPE_BYTES`, this
/// includes all sub-values concatenated.
pub fn decode_column(data: &[u8], expected_crc32: u32) -> TelemetryResult<Vec<f64>> {
    // Verify CRC32 over the entire data
    let actual_crc = crc32fast::hash(data);
    if actual_crc != expected_crc32 {
        return Err(TelemetryError::InvalidFormat(format!(
            "CRC32 mismatch: expected 0x{expected_crc32:08X}, got 0x{actual_crc:08X}"
        )));
    }

    let (codec, value_type, value_count) = read_header(data)?;
    let payload = &data[HEADER_BYTES..];

    match codec {
        CODEC_PLAIN => decode_plain_payload(payload, value_type, value_count),
        CODEC_DELTA => decode_delta_payload(payload, value_type, value_count),
        _ => Err(TelemetryError::InvalidFormat(format!(
            "unknown codec: 0x{codec:02X}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Payload decoding – PLAIN
// ---------------------------------------------------------------------------

fn decode_plain_payload(
    payload: &[u8],
    value_type: u8,
    value_count: u32,
) -> TelemetryResult<Vec<f64>> {
    match value_type {
        TYPE_U64 => decode_fixed::<8>(payload, value_count, |b| u64::from_le_bytes(b) as f64),
        TYPE_I32 => decode_fixed::<4>(payload, value_count, |b| i32::from_le_bytes(b) as f64),
        TYPE_F32 => decode_fixed::<4>(payload, value_count, |b| f32::from_le_bytes(b) as f64),
        TYPE_F64 => decode_fixed::<8>(payload, value_count, f64::from_le_bytes),
        TYPE_BYTES => decode_bytes_plain(payload, value_count),
        TYPE_BYTES_F32 => decode_bytes_plain_typed::<4>(payload, value_count, |b| f32::from_le_bytes(b) as f64),
        TYPE_BYTES_U16 => decode_bytes_plain_typed::<2>(payload, value_count, |b| u16::from_le_bytes(b) as f64),
        TYPE_BYTES_I32 => decode_bytes_plain_typed::<4>(payload, value_count, |b| i32::from_le_bytes(b) as f64),
        _ => Err(TelemetryError::InvalidFormat(format!(
            "unknown value_type: 0x{value_type:02X}"
        ))),
    }
}

/// Decode fixed-size plain elements.
fn decode_fixed<const N: usize>(
    payload: &[u8],
    value_count: u32,
    convert: impl Fn([u8; N]) -> f64,
) -> TelemetryResult<Vec<f64>> {
    let expected_len = value_count as usize * N;
    if payload.len() < expected_len {
        return Err(TelemetryError::InvalidFormat(format!(
            "payload too short for {value_count} values of {N} bytes: got {} bytes",
            payload.len()
        )));
    }
    let mut out = Vec::with_capacity(value_count as usize);
    for chunk in payload[..expected_len].chunks_exact(N) {
        let arr: [u8; N] = chunk.try_into().unwrap();
        out.push(convert(arr));
    }
    Ok(out)
}

/// Decode TYPE_BYTES plain payload.
/// Each item: [sub_value_count: u8] [sub_count × 8 bytes f64 LE]
fn decode_bytes_plain(payload: &[u8], item_count: u32) -> TelemetryResult<Vec<f64>> {
    let mut out = Vec::new();
    let mut pos = 0;
    for _ in 0..item_count {
        if pos >= payload.len() {
            return Err(TelemetryError::InvalidFormat(
                "unexpected end of BYTES payload".to_string(),
            ));
        }
        let sub_count = payload[pos] as usize;
        pos += 1;
        let byte_len = sub_count * 8;
        if pos + byte_len > payload.len() {
            return Err(TelemetryError::InvalidFormat(format!(
                "BYTES payload too short: need {byte_len} bytes at offset {pos}, have {}",
                payload.len()
            )));
        }
        for chunk in payload[pos..pos + byte_len].chunks_exact(8) {
            let arr: [u8; 8] = chunk.try_into().unwrap();
            out.push(f64::from_le_bytes(arr));
        }
        pos += byte_len;
    }
    Ok(out)
}

/// Decode typed BYTES variant plain payload.
/// Each item: [sub_value_count: u8] [sub_count × N bytes LE per value]
fn decode_bytes_plain_typed<const N: usize>(
    payload: &[u8],
    item_count: u32,
    convert: impl Fn([u8; N]) -> f64,
) -> TelemetryResult<Vec<f64>> {
    let mut out = Vec::new();
    let mut pos = 0;
    for _ in 0..item_count {
        if pos >= payload.len() {
            return Err(TelemetryError::InvalidFormat(
                "unexpected end of BYTES payload".to_string(),
            ));
        }
        let sub_count = payload[pos] as usize;
        pos += 1;
        let byte_len = sub_count * N;
        if pos + byte_len > payload.len() {
            return Err(TelemetryError::InvalidFormat(format!(
                "BYTES payload too short: need {byte_len} bytes at offset {pos}, have {}",
                payload.len()
            )));
        }
        for chunk in payload[pos..pos + byte_len].chunks_exact(N) {
            let arr: [u8; N] = chunk.try_into().unwrap();
            out.push(convert(arr));
        }
        pos += byte_len;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Payload decoding – DELTA
// ---------------------------------------------------------------------------

fn decode_delta_payload(
    payload: &[u8],
    value_type: u8,
    value_count: u32,
) -> TelemetryResult<Vec<f64>> {
    if value_count == 0 {
        return Ok(Vec::new());
    }

    match value_type {
        TYPE_U64 => decode_delta_u64(payload, value_count),
        TYPE_I32 => decode_delta_i32(payload, value_count),
        // Floats fall back to PLAIN when encoded; just decode as PLAIN.
        TYPE_F32 | TYPE_F64 => decode_plain_payload(payload, value_type, value_count),
        TYPE_BYTES => decode_bytes_delta(payload, value_count),
        // Typed BYTES variants fall back to PLAIN decode
        TYPE_BYTES_F32 | TYPE_BYTES_U16 | TYPE_BYTES_I32 => {
            decode_plain_payload(payload, value_type, value_count)
        }
        _ => Err(TelemetryError::InvalidFormat(format!(
            "unsupported value_type for DELTA: 0x{value_type:02X}"
        ))),
    }
}

/// Decode delta-u64: first=8 bytes u64 LE, then 8-byte i64 deltas LE.
fn decode_delta_u64(payload: &[u8], value_count: u32) -> TelemetryResult<Vec<f64>> {
    if payload.len() < 8 {
        return Err(TelemetryError::InvalidFormat(
            "DELTA-u64 payload too short".to_string(),
        ));
    }
    let first = u64::from_le_bytes(payload[..8].try_into().unwrap());
    let mut out = Vec::with_capacity(value_count as usize);
    out.push(first as f64);

    let mut current = first as i64;
    let count = value_count as usize - 1;
    let expected_len = count * 8;
    let end = (payload.len() - 8).min(expected_len);
    for chunk in payload[8..8 + end].chunks_exact(8) {
        if chunk.len() < 8 {
            break;
        }
        let delta = i64::from_le_bytes(chunk.try_into().unwrap());
        current = current.wrapping_add(delta);
        out.push(current as f64);
    }
    Ok(out)
}

/// Decode delta-i32: first=4 bytes i32 LE, then 4-byte i32 deltas LE.
fn decode_delta_i32(payload: &[u8], value_count: u32) -> TelemetryResult<Vec<f64>> {
    if payload.len() < 4 {
        return Err(TelemetryError::InvalidFormat(
            "DELTA-i32 payload too short".to_string(),
        ));
    }
    let first = i32::from_le_bytes(payload[..4].try_into().unwrap());
    let mut out = Vec::with_capacity(value_count as usize);
    out.push(first as f64);

    let mut current = first;
    let count = value_count as usize - 1;
    let expected_len = count * 4;
    let end = (payload.len() - 4).min(expected_len);
    for chunk in payload[4..4 + end].chunks_exact(4) {
        if chunk.len() < 4 {
            break;
        }
        let delta = i32::from_le_bytes(chunk.try_into().unwrap());
        current = current.wrapping_add(delta);
        out.push(current as f64);
    }
    Ok(out)
}

/// Decode TYPE_BYTES DELTA payload.
/// First item: [sub_count: u8] [sub_count × 8 bytes f64 LE]
/// Subsequent items: [sub_count: u8] [sub_count × 4 bytes f32 delta LE]
fn decode_bytes_delta(payload: &[u8], item_count: u32) -> TelemetryResult<Vec<f64>> {
    if item_count == 0 {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let mut pos = 0;

    // First item: read sub_count + full 8-byte values
    if pos >= payload.len() {
        return Err(TelemetryError::InvalidFormat(
            "BYTES DELTA payload too short for first item".to_string(),
        ));
    }
    let sub_count = payload[pos] as usize;
    pos += 1;
    let first_byte_len = sub_count * 8;
    if pos + first_byte_len > payload.len() {
        return Err(TelemetryError::InvalidFormat(
            "BYTES DELTA first item payload too short".to_string(),
        ));
    }
    let mut prev: Vec<f64> = Vec::with_capacity(sub_count);
    for chunk in payload[pos..pos + first_byte_len].chunks_exact(8) {
        let arr: [u8; 8] = chunk.try_into().unwrap();
        let v = f64::from_le_bytes(arr);
        out.push(v);
        prev.push(v);
    }
    pos += first_byte_len;

    // Subsequent items: deltas
    for _ in 1..item_count {
        if pos >= payload.len() {
            return Err(TelemetryError::InvalidFormat(
                "BYTES DELTA payload too short for delta item".to_string(),
            ));
        }
        let sc = payload[pos] as usize;
        pos += 1;
        if sc != sub_count {
            return Err(TelemetryError::InvalidFormat(format!(
                "BYTES DELTA sub_count mismatch: expected {sub_count}, got {sc}"
            )));
        }
        let delta_byte_len = sub_count * 4;
        if pos + delta_byte_len > payload.len() {
            return Err(TelemetryError::InvalidFormat(
                "BYTES DELTA item payload too short".to_string(),
            ));
        }
        for (i, prev_value) in prev.iter_mut().enumerate().take(sub_count) {
            let start = pos + i * 4;
            let d = f32::from_le_bytes([
                payload[start],
                payload[start + 1],
                payload[start + 2],
                payload[start + 3],
            ]);
            let cur = *prev_value + d as f64;
            out.push(cur);
            *prev_value = cur;
        }
        pos += delta_byte_len;
    }

    Ok(out)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Roundtrip: all 5 types with PLAIN
    // -----------------------------------------------------------------------

    #[test]
    fn roundtrip_u64_plain() {
        let values: Vec<f64> = vec![0.0, 1.0, u64::MAX as f64, 42.0, 1000.0];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_U64, CODEC_PLAIN, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn roundtrip_i32_plain() {
        let values: Vec<f64> = vec![0.0, -1.0, i32::MAX as f64, i32::MIN as f64, 42.0];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_I32, CODEC_PLAIN, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn roundtrip_f32_plain() {
        let values: Vec<f64> = vec![0.0, 1.5, -3.14, f32::MAX as f64, f32::MIN as f64];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_F32, CODEC_PLAIN, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        // f32 roundtrip loses precision; compare with tolerance
        for (a, b) in values.iter().zip(decoded.iter()) {
            let diff = (a - b).abs();
            assert!(diff < 0.001, "f32 roundtrip mismatch: {a} vs {b}, diff {diff}");
        }
    }

    #[test]
    fn roundtrip_f64_plain() {
        let values: Vec<f64> = vec![0.0, 1.5, -3.14, f64::MAX, f64::MIN, 42.123456789];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_F64, CODEC_PLAIN, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn roundtrip_bytes_plain_sub3() {
        // velocity[3]: 2 frames
        let values: Vec<f64> = vec![10.0, 20.0, 30.0, 15.0, 25.0, 35.0];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_BYTES, CODEC_PLAIN, 3);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn roundtrip_bytes_plain_sub4() {
        // wheelLoad[4]: 3 frames
        let values: Vec<f64> = vec![
            1.0, 2.0, 3.0, 4.0,
            5.0, 6.0, 7.0, 8.0,
            9.0, 10.0, 11.0, 12.0,
        ];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_BYTES, CODEC_PLAIN, 4);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn roundtrip_bytes_plain_sub12() {
        // tyreTemp[12]: 1 frame
        let values: Vec<f64> = (0..12).map(|i| (i * 10) as f64).collect();
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_BYTES, CODEC_PLAIN, 12);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    // -----------------------------------------------------------------------
    // CRC32 corruption detection
    // -----------------------------------------------------------------------

    #[test]
    fn crc32_corruption_detected() {
        let values: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let (mut encoded, crc, _, _) = encode_column(&values, TYPE_U64, CODEC_PLAIN, 0);

        // Flip a byte in the payload
        encoded[HEADER_BYTES + 2] ^= 0xFF;

        let result = decode_column(&encoded, crc);
        assert!(
            result.is_err(),
            "should detect CRC32 corruption but got: {result:?}"
        );
    }

    #[test]
    fn crc32_valid_with_correct_data() {
        let values: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let (encoded, crc, _, _) = encode_column(&values, TYPE_U64, CODEC_PLAIN, 0);

        let result = decode_column(&encoded, crc);
        assert!(result.is_ok(), "valid data should decode: {result:?}");
    }

    #[test]
    fn crc32_wrong_expected() {
        let values: Vec<f64> = vec![1.0, 2.0, 3.0];
        let (encoded, crc, _, _) = encode_column(&values, TYPE_F64, CODEC_PLAIN, 0);

        let result = decode_column(&encoded, crc.wrapping_add(1));
        assert!(result.is_err(), "wrong expected CRC32 should fail");
    }

    // -----------------------------------------------------------------------
    // Delta encoding correctness
    // -----------------------------------------------------------------------

    #[test]
    fn delta_u64_roundtrip() {
        let values: Vec<f64> = vec![100.0, 110.0, 115.0, 200.0, 180.0];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_U64, CODEC_DELTA, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn delta_i32_roundtrip() {
        let values: Vec<f64> = vec![0.0, -5.0, 10.0, -20.0, 30.0];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_I32, CODEC_DELTA, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn delta_u64_large_values() {
        // f64 can exactly represent integers up to 2^53 ≈ 9e15,
        // so keep values within that range for lossless roundtrip.
        let values: Vec<f64> = vec![
            1_000_000_000_000_000.0,
            1_000_000_000_000_100.0,
            1_000_000_000_000_500.0,
            1_000_000_000_001_000.0,
            1_000_000_000_000_000.0, // wrap-around delta test
        ];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_U64, CODEC_DELTA, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn delta_f64_falls_back_to_plain() {
        // f64 with DELTA codec should fall back to PLAIN
        let values: Vec<f64> = vec![1.5, 2.5, 3.5, 4.5];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_F64, CODEC_DELTA, 0);
        // Decode as DELTA: since f64 falls back to PLAIN, decoding as PLAIN should match
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn delta_bytes_roundtrip() {
        // velocity[3] DELTA: 3 frames
        let values: Vec<f64> = vec![
            10.0, 20.0, 30.0,
            12.0, 22.0, 31.5,
            14.0, 24.0, 33.0,
        ];
        let (encoded, crc, _min, _max) = encode_column(&values, TYPE_BYTES, CODEC_DELTA, 3);
        let decoded = decode_column(&encoded, crc).unwrap();
        // f32 deltas lose some precision
        for (a, b) in values.iter().zip(decoded.iter()) {
            let diff = (a - b).abs();
            assert!(diff < 0.01, "DELTA BYTES mismatch: {a} vs {b}, diff {diff}");
        }
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn empty_values() {
        let values: Vec<f64> = vec![];
        let (encoded, crc, min, max) = encode_column(&values, TYPE_U64, CODEC_PLAIN, 0);
        assert_eq!(min, 0.0);
        assert_eq!(max, 0.0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn single_value() {
        let values: Vec<f64> = vec![42.0];
        let (encoded, crc, min, max) = encode_column(&values, TYPE_F64, CODEC_PLAIN, 0);
        assert_eq!(min, 42.0);
        assert_eq!(max, 42.0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn all_zeros() {
        let values: Vec<f64> = vec![0.0; 100];
        for &vt in &[TYPE_U64, TYPE_I32, TYPE_F32, TYPE_F64] {
            let (encoded, crc, _, _) = encode_column(&values, vt, CODEC_PLAIN, 0);
            let decoded = decode_column(&encoded, crc).unwrap();
            for (a, b) in values.iter().zip(decoded.iter()) {
                let diff = (a - b).abs();
                assert!(diff < 0.001, "all-zeros mismatch for vt={vt}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn all_negative() {
        let values: Vec<f64> = vec![-1.0, -2.5, -100.0, -0.001];
        let (encoded, crc, _, _) = encode_column(&values, TYPE_F64, CODEC_PLAIN, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn very_large_values() {
        let values: Vec<f64> = vec![f64::MAX, f64::MIN, 1e308, -1e308];
        let (encoded, crc, _, _) = encode_column(&values, TYPE_F64, CODEC_PLAIN, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn delta_single_value() {
        let values: Vec<f64> = vec![42.0];
        let (encoded, crc, _, _) = encode_column(&values, TYPE_U64, CODEC_DELTA, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn delta_empty_values() {
        let values: Vec<f64> = vec![];
        let (encoded, crc, _, _) = encode_column(&values, TYPE_U64, CODEC_DELTA, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert!(decoded.is_empty());
    }

    // -----------------------------------------------------------------------
    // Min/max tracking
    // -----------------------------------------------------------------------

    #[test]
    fn min_max_tracking() {
        let values: Vec<f64> = vec![5.0, 1.0, 10.0, -3.0, 7.0];
        let (_encoded, _crc, min, max) = encode_column(&values, TYPE_F64, CODEC_PLAIN, 0);
        assert_eq!(min, -3.0);
        assert_eq!(max, 10.0);
    }

    #[test]
    fn min_max_bytes() {
        let values: Vec<f64> = vec![5.0, 1.0, 3.0, 10.0, -3.0, 7.0];
        let (_encoded, _crc, min, max) = encode_column(&values, TYPE_BYTES, CODEC_PLAIN, 3);
        assert_eq!(min, -3.0);
        assert_eq!(max, 10.0);
    }

    // -----------------------------------------------------------------------
    // Unknown codec / value_type
    // -----------------------------------------------------------------------

    #[test]
    fn decode_unknown_codec() {
        let mut data = vec![0xFF, TYPE_U64, 1, 0, 0, 0];
        data.extend_from_slice(&42u64.to_le_bytes());
        let crc = crc32fast::hash(&data);
        let result = decode_column(&data, crc);
        assert!(
            result.is_err(),
            "unknown codec should error, got {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Header validation
    // -----------------------------------------------------------------------

    #[test]
    fn decode_too_short_header() {
        let data = vec![0x00]; // only 1 byte, need 6
        let result = decode_column(&data, crc32fast::hash(&data));
        assert!(result.is_err());
    }

    #[test]
    fn decode_value_count_zero() {
        let values: Vec<f64> = vec![];
        let (encoded, crc, _, _) = encode_column(&values, TYPE_U64, CODEC_PLAIN, 0);
        let decoded = decode_column(&encoded, crc).unwrap();
        assert!(decoded.is_empty());
    }
}
