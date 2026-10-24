#![forbid(unsafe_code)]
//! Zero-crate civil calendar and solar/lunar ephemeris calculator for `45.7°N, 123.9°W`.
//!
//! Implements Howard Hinnant's proleptic Gregorian calendar conversion alongside
//! NOAA/Meeus solar declination, equation of time, daylight duration, and synodic lunar phase.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::EphemerisState;

/// Unix timestamp of the J2000.0 epoch (`2000-01-01 12:00:00 UTC`).
const J2000_UNIX_SEC: f64 = 946_728_000.0;

/// Unix timestamp of the reference astronomical New Moon (`2000-01-06 18:14:00 UTC`).
const REF_NEW_MOON_UNIX_SEC: f64 = 947_182_440.0;

/// Mean synodic lunar month in days (`29.530588853`).
const SYNODIC_MONTH_DAYS: f64 = 29.530_588_853;

/// Standard apparent sunrise/sunset solar altitude angle in degrees (refraction + solar semi-diameter).
const SUNRISE_ALTITUDE_DEG: f64 = -0.833;

/// Returns the current system Unix epoch timestamp in seconds.
#[must_use]
pub fn unix_now_sec() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1_772_000_000)
}

/// Converts days since `1970-01-01` into `(year, month, day)` using Howard Hinnant's algorithm.
#[must_use]
pub const fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d)
}

/// Formats a Unix timestamp in seconds as `"YYYY-MM-DD HH:MMZ"`.
#[must_use]
pub fn format_utc_hm(unix_sec: u64) -> String {
    let days = (unix_sec / 86_400) as i64;
    let rem = unix_sec % 86_400;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}Z")
}

/// Formats a Unix timestamp in seconds as an ISO-8601 date `"YYYY-MM-DD"`.
#[must_use]
pub fn format_iso_date(unix_sec: u64) -> String {
    let days = (unix_sec / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Computes solar and lunar ephemeris state for `(lat_deg, lon_deg)` at `unix_sec`.
#[must_use]
pub fn compute_ephemeris(unix_sec: u64, lat_deg: f64, lon_deg: f64) -> EphemerisState {
    let (delta_rad, eq_time_min) = solar_declination_and_eot(unix_sec);
    let lat_rad = lat_deg.to_radians();

    let utc_minutes = ((unix_sec % 86_400) as f64) / 60.0;
    let tst_min = (utc_minutes + eq_time_min + 4.0 * lon_deg).rem_euclid(1440.0);
    let ha_rad = (tst_min / 4.0 - 180.0).to_radians();

    let sin_el = (lat_rad.sin() * delta_rad.sin()
        + lat_rad.cos() * delta_rad.cos() * ha_rad.cos())
    .clamp(-1.0, 1.0);
    let solar_elevation_deg = sin_el.asin().to_degrees() as f32;

    let daylight_sec = daylight_duration_sec(lat_rad, delta_rad);
    let (prev_delta_rad, _) = solar_declination_and_eot(unix_sec.saturating_sub(86_400));
    let prev_daylight_sec = daylight_duration_sec(lat_rad, prev_delta_rad);
    let daylight_delta_sec = (i64::from(daylight_sec) - i64::from(prev_daylight_sec)) as i32;

    let half_day_min = (f64::from(daylight_sec) / 120.0).clamp(60.0, 660.0);
    let sunrise_min = 720.0 - half_day_min;
    let sunset_min = 720.0 + half_day_min;

    let diurnal_progress = if tst_min >= sunrise_min && tst_min <= sunset_min {
        ((tst_min - sunrise_min) / (2.0 * half_day_min)).clamp(0.0, 1.0) as f32
    } else {
        let night_len = (1440.0 - 2.0 * half_day_min).max(1.0);
        let elapsed_night = if tst_min > sunset_min {
            tst_min - sunset_min
        } else {
            (1440.0 - sunset_min) + tst_min
        };
        (elapsed_night / night_len).clamp(0.0, 1.0) as f32
    };

    let days_since_new_moon = (unix_sec as f64 - REF_NEW_MOON_UNIX_SEC) / 86_400.0;
    let phase_f64 = (days_since_new_moon / SYNODIC_MONTH_DAYS).rem_euclid(1.0);
    let lunar_phase_norm = phase_f64 as f32;
    let lunar_illumination_pct =
        (0.5 * (1.0 - (core::f64::consts::TAU * phase_f64).cos()) * 100.0).clamp(0.0, 100.0)
            as f32;

    EphemerisState {
        daylight_sec,
        daylight_delta_sec,
        solar_elevation_deg,
        diurnal_progress,
        lunar_illumination_pct,
        lunar_phase_norm,
    }
}

/// Computes `(solar_declination_rad, equation_of_time_minutes)` for `unix_sec`.
fn solar_declination_and_eot(unix_sec: u64) -> (f64, f64) {
    let d = (unix_sec as f64 - J2000_UNIX_SEC) / 86_400.0;
    let g_deg = (357.5291 + 0.985_600_28 * d).rem_euclid(360.0);
    let q_deg = (280.459 + 0.985_647_36 * d).rem_euclid(360.0);
    let g_rad = g_deg.to_radians();
    let q_rad = q_deg.to_radians();

    let lambda_deg =
        (q_deg + 1.915 * g_rad.sin() + 0.020 * (2.0 * g_rad).sin()).rem_euclid(360.0);
    let eps_deg = 23.439 - 0.000_000_36 * d;
    let eps_rad = eps_deg.to_radians();

    let sin_delta = (eps_rad.sin() * lambda_deg.to_radians().sin()).clamp(-1.0, 1.0);
    let delta_rad = sin_delta.asin();

    let y = (eps_rad * 0.5).tan().powi(2);
    let e = 0.016_708_6;
    let eot_rad = y * (2.0 * q_rad).sin() - 2.0 * e * g_rad.sin()
        + 4.0 * e * y * g_rad.sin() * (2.0 * q_rad).cos()
        - 0.5 * y * y * (4.0 * q_rad).sin()
        - 1.25 * e * e * (2.0 * g_rad).sin();
    let eq_time_min = 4.0 * eot_rad.to_degrees();

    (delta_rad, eq_time_min)
}

/// Computes total daylight duration in seconds for latitude `lat_rad` and declination `delta_rad`.
fn daylight_duration_sec(lat_rad: f64, delta_rad: f64) -> u32 {
    let denom = lat_rad.cos() * delta_rad.cos();
    if denom.abs() < 1e-9 {
        return if lat_rad * delta_rad >= 0.0 {
            86_400
        } else {
            0
        };
    }
    let cos_omega0 =
        (SUNRISE_ALTITUDE_DEG.to_radians().sin() - lat_rad.sin() * delta_rad.sin()) / denom;
    let omega0_deg = cos_omega0.clamp(-1.0, 1.0).acos().to_degrees();
    (2.0 * omega0_deg * 240.0).round().clamp(0.0, 86_400.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{STATION_LAT_DEG, STATION_LON_DEG};

    #[test]
    fn civil_calendar_formatting_is_exact() {
        assert_eq!(format_utc_hm(0), "1970-01-01 00:00Z");
        assert_eq!(format_utc_hm(947_182_440), "2000-01-06 18:14Z");
        assert_eq!(format_iso_date(947_182_440), "2000-01-06");
    }

    #[test]
    fn ephemeris_bounds_and_seasonal_behavior() {
        // Summer solstice 2026-06-21 ~20:00 UTC (near local solar noon at 123.9W)
        let summer = compute_ephemeris(1_782_072_000, STATION_LAT_DEG, STATION_LON_DEG);
        // Winter solstice 2026-12-21 ~20:00 UTC
        let winter = compute_ephemeris(1_797_883_200, STATION_LAT_DEG, STATION_LON_DEG);

        assert!(summer.daylight_sec > 15 * 3600);
        assert!(winter.daylight_sec < 9 * 3600 + 1800);
        assert!(summer.solar_elevation_deg > winter.solar_elevation_deg);
        assert!((0.0..=1.0).contains(&summer.diurnal_progress));
        assert!((0.0..1.0).contains(&summer.lunar_phase_norm));
        assert!((0.0..=100.0).contains(&summer.lunar_illumination_pct));
    }
}
