#![forbid(unsafe_code)]
//! Fixed-size 512-byte binary telemetry persistence and 64-bit FNV-1a integrity verification.
//!
//! Stores Last-Known-Good (LKG) marine and seismic readings alongside a 28-slot
//! chronological ring buffer of `RingSample` records in `assets/telemetry.bin`.

use std::fs;
use std::io;
use std::path::Path;

use crate::model::{FeedStatus, MarineReading, RingSample, SeismicSummary, TideTrend, RING_CAPACITY};

/// 4-byte file header magic identifying North Oregon Coast telemetry frames.
pub const MAGIC: [u8; 4] = *b"PNW1";

/// Binary schema version (`1`).
pub const SCHEMA_VERSION: u32 = 1;

/// Total serialized frame length in bytes (`16` header + `496` payload = `512` bytes, 1 disk sector).
pub const FRAME_BYTES: usize = 512;

/// Byte offset where the checksummed payload begins.
const PAYLOAD_OFFSET: usize = 16;

/// 64-bit FNV-1a offset basis (`0xcbf29ce484222325`).
const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// 64-bit FNV-1a prime (`0x00000100000001b3`).
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Computes the 64-bit FNV-1a hash of a byte slice.
#[must_use]
pub fn compute_fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV1A_OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

/// Returns realistic North Oregon Coast baseline marine defaults (`6.2 ft @ 13s`, `1018.4 hPa`).
#[must_use]
pub const fn default_marine() -> MarineReading {
    MarineReading {
        swell_height_ft: 6.2,
        swell_period_sec: 13.0,
        pressure_hpa: 1018.4,
        water_temp_f: 51.1,
        tide_mllw_ft: 5.8,
        tide_trend: TideTrend::Flood,
        wind_speed_kt: 11.0,
        wind_dir_ascii: *b"NW ",
        status: FeedStatus::Cached,
    }
}

/// Returns realistic Cascadia baseline micro-seismic defaults (`2 events`, `M1.4 @ 18.2 km`).
#[must_use]
pub const fn default_seismic() -> SeismicSummary {
    SeismicSummary {
        event_count: 2,
        max_magnitude: 1.4,
        latest_depth_km: 18.2,
        secondary_depth_km: 29.4,
        status: FeedStatus::Cached,
    }
}

/// Synthesizes a smooth 28-sample history curve of wave heights (`4.8..6.4 ft`)
/// and barometric pressures (`1015.0..1018.6 hPa`) spaced at 6-hour intervals.
#[must_use]
pub fn seed_history(now_sec: u64) -> Vec<RingSample> {
    let mut samples = Vec::with_capacity(RING_CAPACITY);
    let step_sec: u64 = 6 * 3600;
    let total_span = (RING_CAPACITY as u64 - 1) * step_sec;
    let start_sec = now_sec.saturating_sub(total_span);

    for i in 0..RING_CAPACITY {
        let t = i as f64 / (RING_CAPACITY - 1) as f64;
        let phase = t * core::f64::consts::TAU;
        let swell_wave = 5.6 + 0.65 * (phase * 1.35 - 0.8).sin() + 0.15 * (phase * 2.7).cos();
        let swell_ft = (swell_wave as f32).clamp(4.8, 6.4);
        let pres_wave = 1016.2 + 1.8 * (t - 0.25) + 0.85 * (phase * 0.9 - 1.1).sin();
        let pressure_hpa = (pres_wave as f32).clamp(1015.0, 1018.6);

        samples.push(RingSample {
            timestamp_sec: start_sec + (i as u64) * step_sec,
            swell_ft,
            pressure_hpa,
        });
    }
    if let Some(last) = samples.last_mut() {
        last.swell_ft = 6.2;
        last.pressure_hpa = 1018.4;
    }
    samples
}

/// Loads and validates `assets/telemetry.hex` (or raw 512-byte frame), or returns realistic
/// North Oregon Coast defaults and a 28-sample history curve if missing or corrupted.
#[must_use]
pub fn load_or_seed(path: &Path, now_sec: u64) -> (MarineReading, SeismicSummary, Vec<RingSample>) {
    if let Ok(bytes) = fs::read(path) {
        if let Some(decoded) = deserialize_frame(&bytes) {
            return decoded;
        }
        if let Some(raw) = decode_hex_frame(&bytes) {
            if let Some(decoded) = deserialize_frame(&raw) {
                return decoded;
            }
        }
    }
    (default_marine(), default_seismic(), seed_history(now_sec))
}

/// Appends `new_sample` (or updates the tail if within 300s of the latest entry),
/// enforces `RING_CAPACITY`, writes `path` as an ASCII-armored 512-byte hex dump,
/// and returns the 64-bit FNV-1a checksum.
///
/// # Errors
///
/// Returns an `io::Error` if parent directory creation or file writing fails.
pub fn push_and_save(
    path: &Path,
    marine: &MarineReading,
    seismic: &SeismicSummary,
    history: &mut Vec<RingSample>,
    new_sample: RingSample,
) -> io::Result<u64> {
    if let Some(last) = history.last_mut() {
        if new_sample.timestamp_sec.saturating_sub(last.timestamp_sec) < 300 {
            *last = new_sample;
        } else {
            history.push(new_sample);
        }
    } else {
        history.push(new_sample);
    }

    while history.len() > RING_CAPACITY {
        history.remove(0);
    }

    let (frame, fnv_hash) = serialize_frame(marine, seismic, history);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, encode_hex_frame(&frame))?;
    Ok(fnv_hash)
}

fn encode_hex_frame(frame: &[u8; FRAME_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(FRAME_BYTES * 2 + 32);
    for chunk in frame.chunks(16) {
        for &b in chunk {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0F) as usize] as char);
        }
        out.push('\n');
    }
    out
}

fn decode_hex_frame(input: &[u8]) -> Option<[u8; FRAME_BYTES]> {
    let mut out = [0u8; FRAME_BYTES];
    let mut idx = 0usize;
    let mut high_nibble: Option<u8> = None;
    for &b in input {
        if b.is_ascii_whitespace() {
            continue;
        }
        let val = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        if let Some(hi) = high_nibble.take() {
            if idx >= FRAME_BYTES {
                return None;
            }
            out[idx] = (hi << 4) | val;
            idx += 1;
        } else {
            high_nibble = Some(val);
        }
    }
    if idx == FRAME_BYTES && high_nibble.is_none() {
        Some(out)
    } else {
        None
    }
}

/// Serializes the LKG telemetry and ring buffer into a 512-byte frame and returns `(frame, fnv1a)`.
#[must_use]
pub fn serialize_frame(
    marine: &MarineReading,
    seismic: &SeismicSummary,
    history: &[RingSample],
) -> ([u8; FRAME_BYTES], u64) {
    let mut buf = [0u8; FRAME_BYTES];
    buf[0..4].copy_from_slice(&MAGIC);
    buf[4..8].copy_from_slice(&SCHEMA_VERSION.to_le_bytes());

    let mut pos = PAYLOAD_OFFSET;
    write_f32(&mut buf, &mut pos, marine.swell_height_ft);
    write_f32(&mut buf, &mut pos, marine.swell_period_sec);
    write_f32(&mut buf, &mut pos, marine.pressure_hpa);
    write_f32(&mut buf, &mut pos, marine.water_temp_f);
    write_f32(&mut buf, &mut pos, marine.tide_mllw_ft);
    buf[pos] = marine.tide_trend.to_byte();
    pos += 1;
    write_f32(&mut buf, &mut pos, marine.wind_speed_kt);
    buf[pos..pos + 3].copy_from_slice(&marine.wind_dir_ascii);
    pos += 3;
    buf[pos] = marine.status.to_byte();
    pos += 1;

    buf[pos..pos + 2].copy_from_slice(&seismic.event_count.to_le_bytes());
    pos += 2;
    write_f32(&mut buf, &mut pos, seismic.max_magnitude);
    write_f32(&mut buf, &mut pos, seismic.latest_depth_km);
    write_f32(&mut buf, &mut pos, seismic.secondary_depth_km);
    buf[pos] = seismic.status.to_byte();
    pos += 1;

    let count = history.len().min(RING_CAPACITY) as u32;
    buf[pos..pos + 4].copy_from_slice(&count.to_le_bytes());
    pos += 4;

    for sample in history.iter().take(RING_CAPACITY) {
        let s_bytes = sample.to_bytes();
        buf[pos..pos + 16].copy_from_slice(&s_bytes);
        pos += 16;
    }

    let fnv = compute_fnv1a(&buf[PAYLOAD_OFFSET..FRAME_BYTES]);
    buf[8..16].copy_from_slice(&fnv.to_le_bytes());
    (buf, fnv)
}

/// Deserializes and verifies a 512-byte binary telemetry frame.
#[must_use]
pub fn deserialize_frame(
    bytes: &[u8],
) -> Option<(MarineReading, SeismicSummary, Vec<RingSample>)> {
    if bytes.len() != FRAME_BYTES {
        return None;
    }
    if bytes[0..4] != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != SCHEMA_VERSION {
        return None;
    }
    let mut hash_bytes = [0u8; 8];
    hash_bytes.copy_from_slice(&bytes[8..16]);
    let expected_hash = u64::from_le_bytes(hash_bytes);
    let actual_hash = compute_fnv1a(&bytes[PAYLOAD_OFFSET..FRAME_BYTES]);
    if expected_hash != actual_hash {
        return None;
    }

    let mut pos = PAYLOAD_OFFSET;
    let swell_height_ft = read_f32(bytes, &mut pos);
    let swell_period_sec = read_f32(bytes, &mut pos);
    let pressure_hpa = read_f32(bytes, &mut pos);
    let water_temp_f = read_f32(bytes, &mut pos);
    let tide_mllw_ft = read_f32(bytes, &mut pos);
    let tide_trend = TideTrend::from_byte(bytes[pos]);
    pos += 1;
    let wind_speed_kt = read_f32(bytes, &mut pos);
    let wind_dir_ascii = [bytes[pos], bytes[pos + 1], bytes[pos + 2]];
    pos += 3;
    let _saved_marine_status = FeedStatus::from_byte(bytes[pos]);
    pos += 1;

    let event_count = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
    pos += 2;
    let max_magnitude = read_f32(bytes, &mut pos);
    let latest_depth_km = read_f32(bytes, &mut pos);
    let secondary_depth_km = read_f32(bytes, &mut pos);
    let _saved_seismic_status = FeedStatus::from_byte(bytes[pos]);
    pos += 1;

    let count = u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
        as usize;
    pos += 4;
    if count == 0 || count > RING_CAPACITY {
        return None;
    }

    let mut history = Vec::with_capacity(count);
    for _ in 0..count {
        let mut slot = [0u8; 16];
        slot.copy_from_slice(&bytes[pos..pos + 16]);
        pos += 16;
        history.push(RingSample::from_bytes(&slot));
    }

    let marine = MarineReading {
        swell_height_ft,
        swell_period_sec,
        pressure_hpa,
        water_temp_f,
        tide_mllw_ft,
        tide_trend,
        wind_speed_kt,
        wind_dir_ascii,
        status: FeedStatus::Cached,
    };
    let seismic = SeismicSummary {
        event_count,
        max_magnitude,
        latest_depth_km,
        secondary_depth_km,
        status: FeedStatus::Cached,
    };

    Some((marine, seismic, history))
}

fn write_f32(buf: &mut [u8], pos: &mut usize, val: f32) {
    buf[*pos..*pos + 4].copy_from_slice(&val.to_le_bytes());
    *pos += 4;
}

fn read_f32(buf: &[u8], pos: &mut usize) -> f32 {
    let val = f32::from_le_bytes([buf[*pos], buf[*pos + 1], buf[*pos + 2], buf[*pos + 3]]);
    *pos += 4;
    val
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_standard_vectors() {
        assert_eq!(compute_fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(compute_fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn roundtrip_frame_and_corruption_detection() {
        let marine = default_marine();
        let seismic = default_seismic();
        let history = seed_history(1_772_000_000);
        let (mut frame, hash) = serialize_frame(&marine, &seismic, &history);
        assert_ne!(hash, 0);

        let (m2, s2, h2) = deserialize_frame(&frame).expect("valid frame");
        assert_eq!(m2, marine);
        assert_eq!(s2, seismic);
        assert_eq!(h2.len(), RING_CAPACITY);

        frame[42] ^= 0xFF;
        assert!(deserialize_frame(&frame).is_none());
    }
}
