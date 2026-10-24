#![forbid(unsafe_code)]
//! Dual-theme precision vector SVG instrument synthesizer (`viewBox="0 0 840 104"`).
//!
//! Renders the 3-cell North Oregon Coast field-station telemetry bar in two
//! palettes with zero external dependencies:
//! - **Dark Theme (`render_dark_svg`)**: "Oregon Coast Fog & Evergreen" mid-dark slate.
//! - **Light Theme (`render_light_svg`)**: "USGS Coastal Survey Vellum" archival daylight.

use std::f32::consts::PI;
use std::fmt::Write;

use crate::model::{FeedStatus, RingSample, TelemetrySnapshot, TideTrend};

/// Color palette tokens shared across the 3-cell instrument layout.
struct ThemePalette {
    bg_top: &'static str,
    bg_mid: &'static str,
    bg_bot: &'static str,
    rail_bg: &'static str,
    outer_stroke: &'static str,
    divider: &'static str,
    rim_accent: &'static str,
    rim_peak_opacity: &'static str,
    header_text: &'static str,
    primary_text: &'static str,
    secondary_text: &'static str,
    muted_text: &'static str,
    sea_glass: &'static str,
    ochre: &'static str,
    solar_core: &'static str,
    solar_halo: &'static str,
    moon_lit: &'static str,
    moon_shadow: &'static str,
    moon_stroke: &'static str,
    pill_bg: &'static str,
    pill_stroke: &'static str,
    grid_line: &'static str,
    fill_top_opacity: &'static str,
}

const DARK_PALETTE: ThemePalette = ThemePalette {
    bg_top: "#1f292c",
    bg_mid: "#182023",
    bg_bot: "#141b1e",
    rail_bg: "#101619",
    outer_stroke: "#2f3e43",
    divider: "#283539",
    rim_accent: "#6bc4a6",
    rim_peak_opacity: "0.42",
    header_text: "#9bb0b5",
    primary_text: "#f2f7f6",
    secondary_text: "#b5c7cb",
    muted_text: "#7b9095",
    sea_glass: "#6bc4a6",
    ochre: "#dfb46b",
    solar_core: "#fde68a",
    solar_halo: "#f5c469",
    moon_lit: "#dce7e9",
    moon_shadow: "#233034",
    moon_stroke: "#485c62",
    pill_bg: "#152324",
    pill_stroke: "#2b4c46",
    grid_line: "#263337",
    fill_top_opacity: "0.28",
};

const LIGHT_PALETTE: ThemePalette = ThemePalette {
    bg_top: "#fafcfb",
    bg_mid: "#f4f8f6",
    bg_bot: "#eff4f2",
    rail_bg: "#e5eeeb",
    outer_stroke: "#cbd6d3",
    divider: "#dde6e3",
    rim_accent: "#1b6b4c",
    rim_peak_opacity: "0.22",
    header_text: "#4b6166",
    primary_text: "#0f1e22",
    secondary_text: "#5a7075",
    muted_text: "#52676c",
    sea_glass: "#1b6b4c",
    ochre: "#b4690e",
    solar_core: "#f59e0b",
    solar_halo: "#d97706",
    moon_lit: "#f8fafc",
    moon_shadow: "#334155",
    moon_stroke: "#64748b",
    pill_bg: "#e3f0ea",
    pill_stroke: "#b8d4c8",
    grid_line: "#d5dfdc",
    fill_top_opacity: "0.18",
};

/// Synthesizes the "Oregon Coast Fog & Evergreen" dark-mode SVG telemetry bar.
#[must_use]
pub fn render_dark_svg(snap: &TelemetrySnapshot) -> String {
    render_instrument_svg(snap, &DARK_PALETTE)
}

/// Synthesizes the "USGS Coastal Survey Vellum" light-mode SVG telemetry bar.
#[must_use]
pub fn render_light_svg(snap: &TelemetrySnapshot) -> String {
    render_instrument_svg(snap, &LIGHT_PALETTE)
}

/// Selects up to 8 evenly spaced historical samples for clean Cubic Bezier synthesis.
fn extract_knots(history: &[RingSample], fallback_swell: f32, fallback_pres: f32) -> Vec<(f32, f32)> {
    if history.is_empty() {
        return vec![(fallback_swell, fallback_pres); 6];
    }
    if history.len() == 1 {
        let s = history[0];
        return vec![(s.swell_ft, s.pressure_hpa); 6];
    }
    let target_knots = history.len().min(8);
    let last_idx = history.len() - 1;
    let mut knots = Vec::with_capacity(target_knots);
    for i in 0..target_knots {
        let idx = (i * last_idx) / (target_knots - 1);
        let sample = history[idx];
        knots.push((sample.swell_ft, sample.pressure_hpa));
    }
    knots
}

/// Builds a smooth Catmull-Rom-to-Cubic-Bezier SVG path (`M ... C ...`) and returns
/// `(stroke_path, area_fill_path, last_x, last_y)`.
fn build_cubic_bezier(values: &[f32], x_min: f32, x_max: f32, y_min: f32, y_max: f32) -> (String, String, f32, f32) {
    let n = values.len().max(2);
    let mut min_v = f32::INFINITY;
    let mut max_v = f32::NEG_INFINITY;
    for &v in values {
        if v < min_v {
            min_v = v;
        }
        if v > max_v {
            max_v = v;
        }
    }
    let span = (max_v - min_v).max(0.6);
    let pad_min = min_v - span * 0.15;
    let pad_span = (max_v - min_v) + span * 0.30;

    let mut pts = Vec::with_capacity(n);
    for (i, &v) in values.iter().enumerate() {
        let t = (i as f32) / ((n - 1) as f32);
        let x = x_min + t * (x_max - x_min);
        let norm = ((v - pad_min) / pad_span).clamp(0.0, 1.0);
        let y = y_max - norm * (y_max - y_min);
        pts.push((x, y));
    }

    let mut path = String::with_capacity(256);
    let _ = write!(path, "M {:.1} {:.1}", pts[0].0, pts[0].1);

    for i in 0..(pts.len() - 1) {
        let p0 = if i == 0 { pts[0] } else { pts[i - 1] };
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = if i + 2 < pts.len() { pts[i + 2] } else { pts[i + 1] };

        let c1x = p1.0 + (p2.0 - p0.0) / 6.0;
        let c1y = (p1.1 + (p2.1 - p0.1) / 6.0).clamp(y_min, y_max);
        let c2x = p2.0 - (p3.0 - p1.0) / 6.0;
        let c2y = (p2.1 - (p3.1 - p1.1) / 6.0).clamp(y_min, y_max);

        let _ = write!(
            path,
            " C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
            c1x, c1y, c2x, c2y, p2.0, p2.1
        );
    }

    let &(last_x, last_y) = pts.last().unwrap_or(&(x_max, (y_min + y_max) * 0.5));
    let mut area = path.clone();
    let _ = write!(area, " L {:.1} {:.1} L {:.1} {:.1} Z", last_x, y_max + 2.0, x_min, y_max + 2.0);
    (path, area, last_x, last_y)
}

/// Evaluates a point along the subducting Juan de Fuca slab quadratic Bezier (`M 2 10 Q 46 18, 92 44`).
fn slab_point_for_depth(depth_km: f32) -> (f32, f32) {
    let t = (depth_km / 45.0).clamp(0.14, 0.90);
    let u = 1.0 - t;
    let x = u * u * 2.0 + 2.0 * u * t * 46.0 + t * t * 92.0;
    let y = u * u * 10.0 + 2.0 * u * t * 18.0 + t * t * 44.0;
    (x, y)
}

/// Computes the illuminated lunar terminator SVG path for a disc of radius `r` at `(cx, cy)`.
fn lunar_lit_path(cx: f32, cy: f32, r: f32, phase_norm: f32) -> String {
    let p = phase_norm.rem_euclid(1.0);
    let rx = (r * (2.0 * PI * p).cos().abs()).max(0.15);
    let top_y = cy - r;
    let bot_y = cy + r;

    let (limb_sweep, term_sweep) = if p <= 0.5 {
        let ts = i32::from(p >= 0.25);
        (1, ts)
    } else {
        let ts = i32::from(p >= 0.75);
        (0, ts)
    };

    format!(
        "M {cx:.1} {top_y:.1} A {r:.1} {r:.1} 0 0 {limb_sweep} {cx:.1} {bot_y:.1} A {rx:.2} {r:.1} 0 0 {term_sweep} {cx:.1} {top_y:.1} Z"
    )
}

/// Renders the full 3-cell SVG instrument bar with the provided theme palette.
fn render_instrument_svg(snap: &TelemetrySnapshot, pal: &ThemePalette) -> String {
    let knots = extract_knots(&snap.history, snap.marine.swell_height_ft, snap.marine.pressure_hpa);
    let swell_vals: Vec<f32> = knots.iter().map(|k| k.0).collect();
    let pres_vals: Vec<f32> = knots.iter().map(|k| k.1).collect();

    let (swell_path, swell_area, end_x, end_y) =
        build_cubic_bezier(&swell_vals, 186.0, 296.0, 28.0, 62.0);
    let (pres_path, _, _, _) = build_cubic_bezier(&pres_vals, 186.0, 296.0, 32.0, 64.0);

    let (tide_arrow, tide_color) = match snap.marine.tide_trend {
        TideTrend::Flood => ("▲", pal.sea_glass),
        TideTrend::Ebb => ("▼", pal.ochre),
    };
    let marine_dot = match snap.marine.status {
        FeedStatus::Live => pal.sea_glass,
        FeedStatus::Cached => pal.ochre,
    };
    let seismic_dot = match snap.seismic.status {
        FeedStatus::Live => pal.ochre,
        FeedStatus::Cached => pal.muted_text,
    };

    let (epi1_x, epi1_y) = slab_point_for_depth(snap.seismic.latest_depth_km);
    let (epi2_x, epi2_y) = slab_point_for_depth(snap.seismic.secondary_depth_km);

    let day_h = snap.ephemeris.daylight_sec / 3600;
    let day_m = (snap.ephemeris.daylight_sec % 3600) / 60;
    let delta_sign = if snap.ephemeris.daylight_delta_sec >= 0 { "+" } else { "-" };
    let delta_abs = snap.ephemeris.daylight_delta_sec.unsigned_abs();
    let delta_m = delta_abs / 60;
    let delta_s = delta_abs % 60;

    // Evaluate diurnal sun position: above horizon (y < 28) by day, below horizon (y > 28) by night.
    let is_daylight = snap.ephemeris.solar_elevation_deg >= -0.833;
    let sun_t = snap.ephemeris.diurnal_progress.clamp(0.06, 0.94);
    let sun_u = 1.0 - sun_t;
    let (sun_x, sun_y, sun_halo_op, sun_core_fill) = if is_daylight {
        // Daylight arch: M 6 28 Q 36 0 66 28 (left=sunrise -> top=noon -> right=sunset)
        let x = sun_u * sun_u * 6.0 + 72.0 * sun_u * sun_t + sun_t * sun_t * 66.0;
        let y = sun_u * sun_u * 28.0 + 0.0 * sun_u * sun_t + sun_t * sun_t * 28.0;
        (x, y, "0.28", pal.solar_core)
    } else {
        // Nocturnal sub-horizon path: M 66 28 Q 36 48 6 28 (right=sunset -> nadir -> left=sunrise)
        let x = sun_u * sun_u * 66.0 + 72.0 * sun_u * sun_t + sun_t * sun_t * 6.0;
        let y = sun_u * sun_u * 28.0 + 96.0 * sun_u * sun_t + sun_t * sun_t * 28.0;
        (x, y, "0.16", pal.ochre)
    };

    let moon_d = lunar_lit_path(86.0, 22.0, 9.0, snap.ephemeris.lunar_phase_norm);
    let fnv_short = (snap.ring_fnv1a_hash & 0xFFFF_FFFF) as u32;
    let wind_dir = snap.marine.wind_dir_str();

    let ThemePalette {
        bg_top, bg_mid, bg_bot, rail_bg, outer_stroke, divider, rim_accent,
        rim_peak_opacity, header_text, primary_text, secondary_text, muted_text,
        sea_glass, ochre, solar_core: _, solar_halo, moon_lit, moon_shadow,
        moon_stroke, pill_bg, pill_stroke, grid_line, fill_top_opacity,
    } = *pal;
    let tide_ft = snap.marine.tide_mllw_ft;
    let swell_ft = snap.marine.swell_height_ft;
    let period_s = snap.marine.swell_period_sec;
    let pres_hpa = snap.marine.pressure_hpa;
    let wind_kt = snap.marine.wind_speed_kt;
    let water_f = snap.marine.water_temp_f;
    let ev_cnt = snap.seismic.event_count;
    let max_mag = snap.seismic.max_magnitude;
    let depth_km = snap.seismic.latest_depth_km;
    let sol_deg = snap.ephemeris.solar_elevation_deg;
    let moon_pct = snap.ephemeris.lunar_illumination_pct;
    let rustc_ver = &snap.rustc_version;
    let micros = snap.compute_micros;
    let sync_utc = &snap.sync_utc_label;

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 840 104" width="840" height="104" role="img" aria-label="North Oregon Coast Field Station Telemetry">
  <defs>
    <linearGradient id="chassis-bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="{bg_top}"/>
      <stop offset="58%" stop-color="{bg_mid}"/>
      <stop offset="100%" stop-color="{bg_bot}"/>
    </linearGradient>
    <linearGradient id="rim-sheen" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%" stop-color="{rim_accent}" stop-opacity="0.05"/>
      <stop offset="28%" stop-color="{rim_accent}" stop-opacity="{rim_peak_opacity}"/>
      <stop offset="50%" stop-color="#ffffff" stop-opacity="0.22"/>
      <stop offset="78%" stop-color="{rim_accent}" stop-opacity="{rim_peak_opacity}"/>
      <stop offset="100%" stop-color="{rim_accent}" stop-opacity="0.05"/>
    </linearGradient>
    <linearGradient id="swell-fill" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="{sea_glass}" stop-opacity="{fill_top_opacity}"/>
      <stop offset="100%" stop-color="{sea_glass}" stop-opacity="0.0"/>
    </linearGradient>
    <clipPath id="chassis-clip">
      <rect x="0.5" y="0.5" width="839" height="103" rx="8"/>
    </clipPath>
  </defs>

  <g clip-path="url(#chassis-clip)">
    <rect x="0.5" y="0.5" width="839" height="103" rx="8" fill="url(#chassis-bg)"/>
    <rect x="0" y="78" width="840" height="26" fill="{rail_bg}"/>
    <line x1="1" y1="1.2" x2="839" y2="1.2" stroke="url(#rim-sheen)" stroke-width="1.2"/>
    <line x1="0" y1="78" x2="840" y2="78" stroke="{divider}" stroke-width="1"/>
    <line x1="312" y1="8" x2="312" y2="72" stroke="{divider}" stroke-width="1"/>
    <line x1="576" y1="8" x2="576" y2="72" stroke="{divider}" stroke-width="1"/>

    <!-- CELL 1: NORTH COAST MARINE (0..312) -->
    <g>
      <circle cx="16" cy="16" r="3" fill="{marine_dot}"/>
      <text x="24" y="19" fill="{header_text}" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, sans-serif" font-size="9.5" font-weight="700" letter-spacing="0.9">NORTH COAST MARINE</text>
      <rect x="236" y="8" width="60" height="15" rx="4" fill="{pill_bg}" stroke="{pill_stroke}" stroke-width="0.8"/>
      <text x="266" y="18.5" text-anchor="middle" fill="{tide_color}" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" font-size="9" font-weight="600">{tide_arrow} {tide_ft:+.1} ft</text>

      <text x="14" y="45" fill="{primary_text}" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, sans-serif" font-size="20" font-weight="700" letter-spacing="-0.3">{swell_ft:.1} ft <tspan fill="{sea_glass}" font-size="14" font-weight="600">@ {period_s:.0}s</tspan></text>
      <text x="14" y="64" fill="{secondary_text}" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" font-size="10">{pres_hpa:.1} hPa · {wind_dir} {wind_kt:.0}kt · {water_f:.0}°F</text>

      <line x1="186" y1="64" x2="296" y2="64" stroke="{grid_line}" stroke-width="0.8"/>
      <path d="{swell_area}" fill="url(#swell-fill)"/>
      <path d="{pres_path}" fill="none" stroke="{ochre}" stroke-width="1.3" stroke-opacity="0.82" stroke-linecap="round"/>
      <path d="{swell_path}" fill="none" stroke="{sea_glass}" stroke-width="2" stroke-linecap="round"/>
      <circle cx="{end_x:.1}" cy="{end_y:.1}" r="4.5" fill="{sea_glass}" fill-opacity="0.25"/>
      <circle cx="{end_x:.1}" cy="{end_y:.1}" r="2.3" fill="{sea_glass}"/>
    </g>

    <!-- CELL 2: CASCADIA SEISMIC (312..576) -->
    <g>
      <circle cx="328" cy="16" r="3" fill="{seismic_dot}"/>
      <text x="336" y="19" fill="{header_text}" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, sans-serif" font-size="9.5" font-weight="700" letter-spacing="0.9">CASCADIA SEISMIC</text>
      <text x="326" y="45" fill="{primary_text}" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, sans-serif" font-size="20" font-weight="700" letter-spacing="-0.3">{ev_cnt} <tspan font-size="13.5" font-weight="600" fill="{secondary_text}">events / 7d</tspan></text>
      <text x="326" y="64" fill="{secondary_text}" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" font-size="10">M{max_mag:.1} peak · {depth_km:.1} km depth</text>

      <g transform="translate(476, 18)">
        <line x1="0" y1="10" x2="88" y2="10" stroke="{grid_line}" stroke-width="0.8"/>
        <line x1="0" y1="42" x2="88" y2="42" stroke="{grid_line}" stroke-width="0.8"/>
        <path d="M 2 10 Q 46 18, 88 44" fill="none" stroke="{muted_text}" stroke-width="1.5" stroke-linecap="round"/>
        <circle cx="{epi2_x:.1}" cy="{epi2_y:.1}" r="2.2" fill="{sea_glass}"/>
        <circle cx="{epi1_x:.1}" cy="{epi1_y:.1}" r="9" fill="none" stroke="{ochre}" stroke-width="0.9" stroke-opacity="0.38"/>
        <circle cx="{epi1_x:.1}" cy="{epi1_y:.1}" r="5" fill="none" stroke="{ochre}" stroke-width="1.1" stroke-opacity="0.68"/>
        <circle cx="{epi1_x:.1}" cy="{epi1_y:.1}" r="2.3" fill="{ochre}"/>
      </g>
    </g>

    <!-- CELL 3: EPHEMERIS · 45.7°N 123.9°W (576..840) -->
    <g>
      <circle cx="592" cy="16" r="3" fill="{solar_halo}"/>
      <text x="600" y="19" fill="{header_text}" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, sans-serif" font-size="9.5" font-weight="700" letter-spacing="0.9">EPHEMERIS · 45.7°N 123.9°W</text>
      <text x="588" y="45" fill="{primary_text}" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, sans-serif" font-size="18.5" font-weight="700" letter-spacing="-0.4">{day_h}h {day_m:02}m <tspan font-size="10.5" font-weight="600" fill="{sea_glass}">{delta_sign}{delta_m}m {delta_s:02}s</tspan></text>
      <text x="588" y="64" fill="{secondary_text}" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" font-size="10">SOL {sol_deg:+.1}° · LUNAR {moon_pct:.0}%</text>

      <g transform="translate(726, 18)">
        <line x1="0" y1="28" x2="72" y2="28" stroke="{grid_line}" stroke-width="1"/>
        <path d="M 6 28 Q 36 0 66 28" fill="none" stroke="{ochre}" stroke-width="1.4" stroke-opacity="0.65" stroke-linecap="round"/>
        <path d="M 6 28 Q 36 48 66 28" fill="none" stroke="{muted_text}" stroke-width="1" stroke-opacity="0.35" stroke-dasharray="2 2" stroke-linecap="round"/>
        <circle cx="{sun_x:.1}" cy="{sun_y:.1}" r="6.0" fill="{solar_halo}" fill-opacity="{sun_halo_op}"/>
        <circle cx="{sun_x:.1}" cy="{sun_y:.1}" r="3.2" fill="{sun_core_fill}" stroke="{solar_halo}" stroke-width="1"/>
        <circle cx="86" cy="22" r="9" fill="{moon_shadow}" stroke="{moon_stroke}" stroke-width="0.9"/>
        <path d="{moon_d}" fill="{moon_lit}"/>
      </g>
    </g>

    <!-- BOTTOM CALIBRATION BEZEL RAIL (y: 78..104) -->
    <g font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" font-size="9.5" fill="{muted_text}">
      <text x="14" y="94">45.7°N 123.9°W · RING_BUF 28×16B [FNV1a: 0x{fnv_short:08X}]</text>
      <text x="420" y="94" text-anchor="middle">RUSTC {rustc_ver} · 0 EXTERNAL CRATES · #![forbid(unsafe_code)]</text>
      <text x="826" y="94" text-anchor="end">PASS: {micros} µs · SYNC {sync_utc}</text>
    </g>
  </g>
  <rect x="0.5" y="0.5" width="839" height="103" rx="8" fill="none" stroke="{outer_stroke}" stroke-width="1"/>
</svg>
"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EphemerisState, MarineReading, SeismicSummary};

    #[test]
    fn renders_both_themes_with_bezier_and_rail() {
        let snap = TelemetrySnapshot {
            marine: MarineReading {
                swell_height_ft: 8.4, swell_period_sec: 13.0, pressure_hpa: 1018.2,
                water_temp_f: 51.0, tide_mllw_ft: 5.8, tide_trend: TideTrend::Flood,
                wind_speed_kt: 14.0, wind_dir_ascii: *b"WNW", status: FeedStatus::Live,
            },
            history: vec![
                RingSample { timestamp_sec: 1, swell_ft: 7.2, pressure_hpa: 1016.0 },
                RingSample { timestamp_sec: 2, swell_ft: 8.4, pressure_hpa: 1018.2 },
            ],
            seismic: SeismicSummary {
                event_count: 14, max_magnitude: 2.4, latest_depth_km: 24.5,
                secondary_depth_km: 16.0, status: FeedStatus::Live,
            },
            ephemeris: EphemerisState {
                daylight_sec: 38040, daylight_delta_sec: 168, solar_elevation_deg: 28.4,
                diurnal_progress: 0.52, lunar_illumination_pct: 78.0, lunar_phase_norm: 0.35,
            },
            ring_fnv1a_hash: 0xA4F1_9C02, rustc_version: String::from("1.84"),
            compute_micros: 142, sync_utc_label: String::from("18:00Z"),
        };
        let dark = render_dark_svg(&snap);
        let light = render_light_svg(&snap);
        assert!(dark.contains("viewBox=\"0 0 840 104\"") && dark.contains("#1f292c") && dark.contains(" C "));
        assert!(light.contains("viewBox=\"0 0 840 104\"") && light.contains("#fafcfb") && light.contains("0xA4F19C02"));
    }
}
