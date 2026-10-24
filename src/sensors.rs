#![forbid(unsafe_code)]
//! Zero-crate upstream telemetry acquisition for NOAA NDBC, NOAA CO-OPS, and USGS FDSN.
//!
//! Executes bounded `curl` subprocesses (`--max-time 4`) and parses tabular NDBC text
//! and JSON payloads in pure Rust standard library with automatic Last-Known-Good fallback.

use std::process::Command;

use crate::ephemeris::format_iso_date;
use crate::model::{FeedStatus, MarineReading, SeismicSummary, TideTrend};

/// User-Agent header identifying the field station to NOAA and USGS endpoints.
const USER_AGENT: &str = "PNWcoastie-FieldStation/0.1 (+https://github.com/PNWcoastie)";

/// Primary Columbia River Bar NDBC buoy (`46029`).
const NDBC_PRIMARY_URL: &str = "https://www.ndbc.noaa.gov/data/realtime2/46029.txt";

/// Secondary Clatsop Spit NDBC wave buoy (`46243`).
const NDBC_FALLBACK_URL: &str = "https://www.ndbc.noaa.gov/data/realtime2/46243.txt";

/// NOAA CO-OPS Station `9437540` (Garibaldi, Tillamook Bay) water-level endpoint.
const COOPS_TIDE_URL: &str = "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?range=2&station=9437540&product=water_level&datum=MLLW&time_zone=gmt&units=english&format=json";

/// 16-point cardinal compass table (3-byte ASCII padded with spaces).
const COMPASS_16: [[u8; 3]; 16] = [
    *b"N  ", *b"NNE", *b"NE ", *b"ENE", *b"E  ", *b"ESE", *b"SE ", *b"SSE", *b"S  ", *b"SSW",
    *b"SW ", *b"WSW", *b"W  ", *b"WNW", *b"NW ", *b"NNW",
];

/// Executes a bounded 4-second HTTP GET via `curl` and returns the UTF-8 response body.
#[must_use]
pub fn http_get(url: &str) -> Option<String> {
    let output = Command::new("curl")
        .args([
            "-sS",
            "--fail",
            "--max-time",
            "4",
            "-A",
            USER_AGENT,
            url,
        ])
        .output()
        .ok()?;

    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Fetches live marine telemetry from NDBC (`46029` / `46243`) and NOAA CO-OPS (`9437540`),
/// falling back field-by-field to `lkg` whenever an upstream column is `MM` or unreachable.
#[must_use]
pub fn fetch_marine(lkg: &MarineReading) -> MarineReading {
    let mut out = *lkg;
    out.status = FeedStatus::Cached;

    let mut parsed_ndbc = NdbcFields::default();
    if let Some(body) = http_get(NDBC_PRIMARY_URL) {
        parsed_ndbc.merge_from(&parse_ndbc_realtime(&body));
    }
    if !parsed_ndbc.has_waves() {
        if let Some(body) = http_get(NDBC_FALLBACK_URL) {
            parsed_ndbc.merge_from(&parse_ndbc_realtime(&body));
        }
    }

    let mut any_live = false;
    if let Some(wvht_m) = parsed_ndbc.wvht_m {
        out.swell_height_ft = wvht_m * 3.280_84;
        any_live = true;
    }
    if let Some(dpd_s) = parsed_ndbc.dpd_sec {
        out.swell_period_sec = dpd_s;
        any_live = true;
    }
    if let Some(pres) = parsed_ndbc.pres_hpa {
        out.pressure_hpa = pres;
        any_live = true;
    }
    if let Some(wtmp_c) = parsed_ndbc.wtmp_c {
        out.water_temp_f = wtmp_c * 1.8 + 32.0;
        any_live = true;
    }
    if let Some(wspd_ms) = parsed_ndbc.wspd_ms {
        out.wind_speed_kt = wspd_ms * 1.943_84;
        any_live = true;
    }
    if let Some(wdir_deg) = parsed_ndbc.wdir_deg {
        out.wind_dir_ascii = degrees_to_cardinal(wdir_deg);
        any_live = true;
    }

    if let Some(coops_json) = http_get(COOPS_TIDE_URL) {
        if let Some((level_ft, trend)) = parse_coops_tide(&coops_json) {
            out.tide_mllw_ft = level_ft;
            out.tide_trend = trend;
            any_live = true;
        }
    }

    if any_live {
        out.status = FeedStatus::Live;
    }
    out
}

/// Fetches 7-day Cascadia micro-seismic activity from USGS FDSN for `44.5..46.8°N, -125.8..-122.5°W`.
#[must_use]
pub fn fetch_seismic(lkg: &SeismicSummary, now_sec: u64) -> SeismicSummary {
    let start_date = format_iso_date(now_sec.saturating_sub(7 * 86_400));
    let url = format!(
        "https://earthquake.usgs.gov/fdsnws/event/1/query?format=geojson&minlatitude=44.5&maxlatitude=46.8&minlongitude=-125.8&maxlongitude=-122.5&starttime={start_date}"
    );

    if let Some(body) = http_get(&url) {
        if let Some(summary) = parse_usgs_geojson(&body, lkg) {
            return summary;
        }
    }
    let mut cached = *lkg;
    cached.status = FeedStatus::Cached;
    cached
}

/// Converts meteorological wind direction in degrees (`0..=360`) into a 3-byte ASCII cardinal label.
#[must_use]
pub fn degrees_to_cardinal(deg: f32) -> [u8; 3] {
    let norm = deg.rem_euclid(360.0);
    let idx = ((norm + 11.25) / 22.5).floor() as usize % 16;
    COMPASS_16[idx]
}

#[derive(Debug, Default, Clone, Copy)]
struct NdbcFields {
    wvht_m: Option<f32>,
    dpd_sec: Option<f32>,
    pres_hpa: Option<f32>,
    wtmp_c: Option<f32>,
    wdir_deg: Option<f32>,
    wspd_ms: Option<f32>,
}

impl NdbcFields {
    const fn has_waves(self) -> bool {
        self.wvht_m.is_some() && self.dpd_sec.is_some()
    }

    fn merge_from(&mut self, other: &Self) {
        if self.wvht_m.is_none() {
            self.wvht_m = other.wvht_m;
        }
        if self.dpd_sec.is_none() {
            self.dpd_sec = other.dpd_sec;
        }
        if self.pres_hpa.is_none() {
            self.pres_hpa = other.pres_hpa;
        }
        if self.wtmp_c.is_none() {
            self.wtmp_c = other.wtmp_c;
        }
        if self.wdir_deg.is_none() {
            self.wdir_deg = other.wdir_deg;
        }
        if self.wspd_ms.is_none() {
            self.wspd_ms = other.wspd_ms;
        }
    }
}

/// Parses an NDBC `realtime2/*.txt` table, scanning recent rows for the newest non-`MM` values.
fn parse_ndbc_realtime(text: &str) -> NdbcFields {
    let mut fields = NdbcFields::default();
    let mut col_wdir = 5usize;
    let mut col_wspd = 6usize;
    let mut col_wvht = 8usize;
    let mut col_dpd = 9usize;
    let mut col_pres = 12usize;
    let mut col_wtmp = 14usize;

    for (row_idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(header) = trimmed.strip_prefix('#') {
            if row_idx == 0 {
                for (i, token) in header.split_whitespace().enumerate() {
                    match token {
                        "WDIR" => col_wdir = i,
                        "WSPD" => col_wspd = i,
                        "WVHT" => col_wvht = i,
                        "DPD" => col_dpd = i,
                        "PRES" | "BAR" => col_pres = i,
                        "WTMP" => col_wtmp = i,
                        _ => {}
                    }
                }
            }
            continue;
        }

        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols.len() <= col_wvht {
            continue;
        }
        if fields.wdir_deg.is_none() {
            fields.wdir_deg = parse_ndbc_val(cols.get(col_wdir).copied());
        }
        if fields.wspd_ms.is_none() {
            fields.wspd_ms = parse_ndbc_val(cols.get(col_wspd).copied());
        }
        if fields.wvht_m.is_none() {
            fields.wvht_m = parse_ndbc_val(cols.get(col_wvht).copied());
        }
        if fields.dpd_sec.is_none() {
            fields.dpd_sec = parse_ndbc_val(cols.get(col_dpd).copied());
        }
        if fields.pres_hpa.is_none() {
            fields.pres_hpa = parse_ndbc_val(cols.get(col_pres).copied());
        }
        if fields.wtmp_c.is_none() {
            fields.wtmp_c = parse_ndbc_val(cols.get(col_wtmp).copied());
        }

        if fields.wvht_m.is_some()
            && fields.dpd_sec.is_some()
            && fields.pres_hpa.is_some()
            && fields.wtmp_c.is_some()
            && fields.wdir_deg.is_some()
            && fields.wspd_ms.is_some()
        {
            break;
        }
    }
    fields
}

fn parse_ndbc_val(token: Option<&str>) -> Option<f32> {
    let raw = token?;
    if raw == "MM" || raw == "99.0" || raw == "99.00" || raw == "999" || raw == "9999.0" {
        return None;
    }
    raw.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Extracts verified MLLW water-level values from NOAA CO-OPS JSON and computes the tide trend.
#[must_use]
pub fn parse_coops_tide(json: &str) -> Option<(f32, TideTrend)> {
    let mut values = Vec::new();
    let mut rest = json;
    while let Some(idx) = rest.find("\"v\"") {
        rest = &rest[idx + 3..];
        let colon = rest.find(':')?;
        rest = &rest[colon + 1..];
        let quote_start = rest.find('"')?;
        rest = &rest[quote_start + 1..];
        let quote_end = rest.find('"')?;
        let val_str = rest[..quote_end].trim();
        if let Ok(v) = val_str.parse::<f32>() {
            if v.is_finite() {
                values.push(v);
            }
        }
        rest = &rest[quote_end + 1..];
    }

    let &latest = values.last()?;
    let trend = if values.len() >= 2 {
        let prev = values[values.len() - 2];
        if latest >= prev {
            TideTrend::Flood
        } else {
            TideTrend::Ebb
        }
    } else {
        TideTrend::Flood
    };
    Some((latest, trend))
}

/// Parses USGS FDSN GeoJSON features to extract event count, peak magnitude, and hypocenter depths.
#[must_use]
pub fn parse_usgs_geojson(json: &str, lkg: &SeismicSummary) -> Option<SeismicSummary> {
    if !json.contains("\"features\"") {
        return None;
    }

    let mut mags = Vec::new();
    let mut search = json;
    while let Some(idx) = search.find("\"mag\":") {
        search = &search[idx + 6..];
        let trimmed = search.trim_start();
        let end = trimmed
            .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
            .unwrap_or(trimmed.len());
        if end > 0 {
            if let Ok(m) = trimmed[..end].parse::<f32>() {
                if m.is_finite() {
                    mags.push(m);
                }
            }
        }
    }

    let mut depths = Vec::new();
    let mut csearch = json;
    while let Some(idx) = csearch.find("\"coordinates\":") {
        csearch = &csearch[idx + 14..];
        if let Some(open_b) = csearch.find('[') {
            let after_open = &csearch[open_b + 1..];
            if let Some(close_b) = after_open.find(']') {
                let coords_str = &after_open[..close_b];
                let mut parts = coords_str.split(',');
                let _lon = parts.next();
                let _lat = parts.next();
                if let Some(depth_raw) = parts.next() {
                    if let Ok(d) = depth_raw.trim().parse::<f32>() {
                        if d.is_finite() {
                            depths.push(d.abs());
                        }
                    }
                }
                csearch = &after_open[close_b + 1..];
            }
        }
    }

    let event_count = depths.len().max(mags.len()).min(u16::MAX as usize) as u16;
    let max_magnitude = mags
        .iter()
        .copied()
        .reduce(f32::max)
        .unwrap_or(lkg.max_magnitude);
    let latest_depth_km = depths.first().copied().unwrap_or(lkg.latest_depth_km);
    let secondary_depth_km = depths
        .get(1)
        .copied()
        .unwrap_or(lkg.secondary_depth_km);

    Some(SeismicSummary {
        event_count,
        max_magnitude,
        latest_depth_km,
        secondary_depth_km,
        status: FeedStatus::Live,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ringbuf::{default_marine, default_seismic};

    #[test]
    fn parses_ndbc_mixed_rows_and_coops_tide() {
        let sample_ndbc = "\
#YY  MM DD hh mm WDIR WSPD GST  WVHT   DPD   APD MWD   PRES  ATMP  WTMP  DEWP  VIS PTDY  TIDE
#yr  mo dy hr mn degT m/s  m/s     m   sec   sec degT   hPa  degC  degC  degC  nmi  hPa    ft
2026 10 24 18 50 310  5.6  7.1    MM    MM    MM  MM 1018.4  11.2  10.6   9.1   MM   MM    MM
2026 10 24 18 40  MM   MM   MM   1.90 13.0   8.4 295 1018.2    MM  10.6    MM   MM   MM    MM
";
        let f = parse_ndbc_realtime(sample_ndbc);
        assert!((f.wvht_m.unwrap() - 1.90).abs() < 1e-4);
        assert!((f.dpd_sec.unwrap() - 13.0).abs() < 1e-4);
        assert!((f.pres_hpa.unwrap() - 1018.4).abs() < 1e-3);
        assert_eq!(degrees_to_cardinal(f.wdir_deg.unwrap()), *b"NW ");

        let coops = r#"{"data":[{"t":"2026-10-24 18:42","v":"5.412"},{"t":"2026-10-24 18:48","v":"5.804"}]}"#;
        let (lvl, trend) = parse_coops_tide(coops).expect("valid coops");
        assert!((lvl - 5.804).abs() < 1e-3);
        assert_eq!(trend, TideTrend::Flood);
        let _ = default_marine();
    }

    #[test]
    fn parses_usgs_geojson_payload() {
        let json = r#"{"type":"FeatureCollection","features":[{"properties":{"mag":1.8},"geometry":{"type":"Point","coordinates":[-124.12,45.65,19.4]}},{"properties":{"mag":1.1},"geometry":{"type":"Point","coordinates":[-123.88,45.82,31.2]}}]}"#;
        let lkg = default_seismic();
        let summary = parse_usgs_geojson(json, &lkg).expect("valid geojson");
        assert_eq!(summary.event_count, 2);
        assert!((summary.max_magnitude - 1.8).abs() < 1e-4);
        assert!((summary.latest_depth_km - 19.4).abs() < 1e-4);
        assert!((summary.secondary_depth_km - 31.2).abs() < 1e-4);
        assert_eq!(summary.status, FeedStatus::Live);
    }
}
