#![forbid(unsafe_code)]
//! `pnwcoastie-telemetry`: Zero-dependency, `#![forbid(unsafe_code)]` North Oregon Coast
//! field-station telemetry engine and dual-theme SVG instrument synthesizer.

pub mod ephemeris;
pub mod model;
pub mod ringbuf;
pub mod sensors;
pub mod svg;

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::model::{
    RingSample, TelemetrySnapshot, STATION_LAT_DEG, STATION_LON_DEG,
};

/// Detects the active `rustc` major.minor version string (e.g. `"1.84"`).
fn detect_rustc_version() -> String {
    let output = Command::new("rustc").arg("--version").output();
    if let Ok(out) = output {
        if out.status.success() {
            if let Ok(text) = String::from_utf8(out.stdout) {
                // Format: "rustc 1.84.0 (..."
                if let Some(ver_token) = text.split_whitespace().nth(1) {
                    let mut parts = ver_token.split('.');
                    if let (Some(maj), Some(min)) = (parts.next(), parts.next()) {
                        return format!("{maj}.{min}");
                    }
                }
            }
        }
    }
    String::from("1.84")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let offline_mode = env::args().any(|arg| arg == "--offline");
    let assets_dir = Path::new("assets");
    fs::create_dir_all(assets_dir)?;

    let ring_path = assets_dir.join("telemetry.hex");
    let dark_svg_path = assets_dir.join("telemetry-dark.svg");
    let light_svg_path = assets_dir.join("telemetry-light.svg");

    let now_sec = ephemeris::unix_now_sec();
    let rustc_version = detect_rustc_version();

    // Load Last-Known-Good (LKG) snapshot and 28-slot ring buffer history.
    let (lkg_marine, lkg_seismic, mut history) = ringbuf::load_or_seed(&ring_path, now_sec);

    // Fetch live upstream NOAA NDBC / CO-OPS and USGS FDSN feeds (with graceful fallback to LKG).
    let (marine, seismic) = if offline_mode {
        (lkg_marine, lkg_seismic)
    } else {
        let m = sensors::fetch_marine(&lkg_marine);
        let s = sensors::fetch_seismic(&lkg_seismic, now_sec);
        (m, s)
    };

    // Start deterministic compute timer for local ephemeris, binary ring buffer, and SVG pass.
    let compute_start = Instant::now();

    let ephem = ephemeris::compute_ephemeris(now_sec, STATION_LAT_DEG, STATION_LON_DEG);
    let sync_utc_label = ephemeris::format_utc_hm(now_sec);

    let new_sample = RingSample {
        timestamp_sec: now_sec,
        swell_ft: marine.swell_height_ft,
        pressure_hpa: marine.pressure_hpa,
    };

    let ring_fnv1a_hash =
        ringbuf::push_and_save(&ring_path, &marine, &seismic, &mut history, new_sample)?;

    let compute_micros = compute_start.elapsed().as_micros() as u64;

    let snapshot = TelemetrySnapshot {
        marine,
        history,
        seismic,
        ephemeris: ephem,
        ring_fnv1a_hash,
        rustc_version,
        compute_micros: compute_micros.max(1),
        sync_utc_label,
    };

    let dark_svg = svg::render_dark_svg(&snapshot);
    let light_svg = svg::render_light_svg(&snapshot);

    fs::write(&dark_svg_path, dark_svg)?;
    fs::write(&light_svg_path, light_svg)?;

    println!(
        "[pnwcoastie-telemetry] OK | swell={:.1}ft@{:.0}s tide={:+.1}ft({:?}) pres={:.1}hPa | seismic={}ev(M{:.1}) | sol={:+.1}deg moon={:.0}% | fnv1a=0x{:08X} | pass={}us",
        snapshot.marine.swell_height_ft,
        snapshot.marine.swell_period_sec,
        snapshot.marine.tide_mllw_ft,
        snapshot.marine.tide_trend,
        snapshot.marine.pressure_hpa,
        snapshot.seismic.event_count,
        snapshot.seismic.max_magnitude,
        snapshot.ephemeris.solar_elevation_deg,
        snapshot.ephemeris.lunar_illumination_pct,
        snapshot.ring_fnv1a_hash & 0xFFFF_FFFF,
        snapshot.compute_micros
    );

    Ok(())
}
