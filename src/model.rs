#![forbid(unsafe_code)]
//! Shared domain data contract for the North Oregon Coast field-station telemetry engine.
//!
//! All modules (`ringbuf`, `sensors`, `ephemeris`, `svg`) operate against these
//! fixed-footprint structures with zero external crate dependencies.

/// Number of historical marine samples retained in the fixed-size binary ring buffer.
pub const RING_CAPACITY: usize = 28;

/// Reference latitude for North Oregon Coast ephemeris & sensor bounding (45.7°N).
pub const STATION_LAT_DEG: f64 = 45.70;

/// Reference longitude for North Oregon Coast ephemeris & sensor bounding (123.9°W).
pub const STATION_LON_DEG: f64 = -123.90;

/// Provenance state of a telemetry cell (Live network fetch vs. Last-Known-Good fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedStatus {
    /// Freshly fetched from upstream NOAA / USGS endpoint during this pass.
    Live,
    /// Upstream timed out or reported missing column (`MM`); served from binary LKG cache.
    Cached,
}

impl FeedStatus {
    /// Encodes status as a single byte for binary serialization.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Live => 1,
            Self::Cached => 0,
        }
    }

    /// Decodes status from a single byte.
    #[must_use]
    pub const fn from_byte(b: u8) -> Self {
        if b == 1 {
            Self::Live
        } else {
            Self::Cached
        }
    }
}

/// Tide directional phase derived from consecutive water-level samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TideTrend {
    /// Rising tide (`▲ FLOOD`).
    Flood,
    /// Falling tide (`▼ EBB`).
    Ebb,
}

impl TideTrend {
    /// Encodes tide trend as a single byte.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Flood => 1,
            Self::Ebb => 0,
        }
    }

    /// Decodes tide trend from a single byte.
    #[must_use]
    pub const fn from_byte(b: u8) -> Self {
        if b == 0 {
            Self::Ebb
        } else {
            Self::Flood
        }
    }
}

/// A single 16-byte historical slot inside the 28-sample marine ring buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RingSample {
    /// Unix epoch timestamp (seconds, truncated to `u64`).
    pub timestamp_sec: u64,
    /// Dominant swell wave height in feet (`WVHT` converted from meters).
    pub swell_ft: f32,
    /// Sea-level barometric pressure in hectopascals (`PRES` in hPa).
    pub pressure_hpa: f32,
}

impl RingSample {
    /// Serializes a single sample into a 16-byte little-endian array.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..8].copy_from_slice(&self.timestamp_sec.to_le_bytes());
        out[8..12].copy_from_slice(&self.swell_ft.to_le_bytes());
        out[12..16].copy_from_slice(&self.pressure_hpa.to_le_bytes());
        out
    }

    /// Deserializes a single sample from a 16-byte little-endian array.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; 16]) -> Self {
        let mut ts = [0u8; 8];
        let mut sw = [0u8; 4];
        let mut pr = [0u8; 4];
        ts.copy_from_slice(&bytes[0..8]);
        sw.copy_from_slice(&bytes[8..12]);
        pr.copy_from_slice(&bytes[12..16]);
        Self {
            timestamp_sec: u64::from_le_bytes(ts),
            swell_ft: f32::from_le_bytes(sw),
            pressure_hpa: f32::from_le_bytes(pr),
        }
    }
}

/// Cell 1: Unified North Coast Marine reading (NDBC Station 46029 + CO-OPS Station 9437540).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarineReading {
    /// Dominant swell height in feet (`46029`).
    pub swell_height_ft: f32,
    /// Dominant wave period in seconds (`46029`).
    pub swell_period_sec: f32,
    /// Sea-level barometric pressure in hPa (`46029`).
    pub pressure_hpa: f32,
    /// Surface water temperature in degrees Fahrenheit (`46029` / `9437540`).
    pub water_temp_f: f32,
    /// Verified tide water level in feet relative to MLLW (`9437540`).
    pub tide_mllw_ft: f32,
    /// Rising (`Flood`) or falling (`Ebb`) tide vector.
    pub tide_trend: TideTrend,
    /// Sustained wind speed in knots.
    pub wind_speed_kt: f32,
    /// Cardinal wind direction string (encoded as 3 ASCII bytes, e.g. `*b"NW "` or `*b"WNW"`).
    pub wind_dir_ascii: [u8; 3],
    /// Live vs. cached status for the marine panel.
    pub status: FeedStatus,
}

impl MarineReading {
    /// Returns the wind direction as a trimmed UTF-8 string slice.
    #[must_use]
    pub fn wind_dir_str(&self) -> &str {
        core::str::from_utf8(&self.wind_dir_ascii)
            .unwrap_or("NW")
            .trim()
    }
}

/// Cell 2: Cascadia / North Oregon Coast micro-seismic summary (USGS FDSN).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeismicSummary {
    /// Number of recorded seismic events in the bounding box over the past 7 days (or 24h window).
    pub event_count: u16,
    /// Peak magnitude among recent events (`M`).
    pub max_magnitude: f32,
    /// Hypocenter depth of the peak/latest event in kilometers.
    pub latest_depth_km: f32,
    /// Secondary event depth in kilometers for subduction cross-section plotting.
    pub secondary_depth_km: f32,
    /// Live vs. cached status for the seismic panel.
    pub status: FeedStatus,
}

/// Cell 3: Zero-network solar & lunar ephemeris computed locally for 45.7°N, 123.9°W.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EphemerisState {
    /// Total daylight duration in seconds today.
    pub daylight_sec: u32,
    /// Change in daylight duration compared to yesterday in signed seconds.
    pub daylight_delta_sec: i32,
    /// Current solar elevation angle above/below the local horizon in degrees.
    pub solar_elevation_deg: f32,
    /// Normalized diurnal sun progress across the sky (`0.0..=1.0`).
    pub diurnal_progress: f32,
    /// Lunar illumination percentage (`0.0..=100.0`).
    pub lunar_illumination_pct: f32,
    /// Normalized synodic lunar phase (`0.0..1.0`, where `0.0` = New, `0.5` = Full).
    pub lunar_phase_norm: f32,
}

/// Complete telemetry snapshot passed to the dual-theme SVG instrument renderer.
#[derive(Debug, Clone)]
pub struct TelemetrySnapshot {
    /// Cell 1 marine reading.
    pub marine: MarineReading,
    /// Chronological marine samples (`<= RING_CAPACITY`) for smooth Bezier curve synthesis.
    pub history: Vec<RingSample>,
    /// Cell 2 Cascadia seismic reading.
    pub seismic: SeismicSummary,
    /// Cell 3 solar & lunar ephemeris state.
    pub ephemeris: EphemerisState,
    /// 64-bit FNV-1a integrity checksum of the binary ring buffer.
    pub ring_fnv1a_hash: u64,
    /// `rustc` version string (e.g., `"1.84"`).
    pub rustc_version: String,
    /// Execution duration of the sensor + ephemeris + ring-buffer pass in microseconds.
    pub compute_micros: u64,
    /// Formatted UTC synchronization timestamp (`"HH:MMZ"` or `"YYYY-MM-DD HH:MMZ"`).
    pub sync_utc_label: String,
}
