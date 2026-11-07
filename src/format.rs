//! Human-friendly formatting for numbers shown in the UI.

const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

/// Formats a byte count using binary (1024) steps, e.g. `1.5 GB`.
pub fn bytes(n: u64) -> String {
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Formats a transfer rate; zero shows as a dash to keep idle rows quiet.
pub fn rate(bytes_per_sec: u64) -> String {
    if bytes_per_sec == 0 {
        "—".into()
    } else {
        format!("{}/s", bytes(bytes_per_sec))
    }
}

/// Like [`rate`], but always shows a number (for headline figures).
pub fn speed(bytes_per_sec: u64) -> String {
    format!("{}/s", bytes(bytes_per_sec))
}

pub fn percent(p: f32) -> String {
    if p < 0.05 {
        "0%".into()
    } else if p < 10.0 {
        format!("{p:.1}%")
    } else {
        format!("{p:.0}%")
    }
}

/// Formats an uptime such as `3d 4h 12m` or `5h 02m`.
pub fn duration(secs: u64) -> String {
    let (days, hours, minutes) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    if days > 0 {
        format!("{days}d {hours}h {minutes:02}m")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m {:02}s", secs % 60)
    }
}

pub fn frequency(mhz: u64) -> String {
    if mhz >= 1000 {
        format!("{:.2} GHz", mhz as f64 / 1000.0)
    } else {
        format!("{mhz} MHz")
    }
}

/// Rounds a rate up to a "nice" graph ceiling (1, 2 or 5 × 1024ⁿ bytes) with headroom,
/// so auto-scaled graphs get stable, readable axis labels.
pub fn nice_ceiling(max_value: f32) -> f32 {
    let target = (max_value * 1.15).max(1024.0);
    let mut magnitude = 1.0_f32;
    while magnitude * 1024.0 <= target {
        magnitude *= 1024.0;
    }
    [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1024.0]
        .into_iter()
        .map(|step| step * magnitude)
        .find(|&v| v >= target)
        .unwrap_or(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_duration_and_frequency() {
        assert_eq!(duration(59), "0m 59s");
        assert_eq!(duration(3 * 3600 + 5 * 60), "3h 05m");
        assert_eq!(duration(2 * 86_400 + 3600), "2d 1h 00m");
        assert_eq!(frequency(3450), "3.45 GHz");
        assert_eq!(frequency(800), "800 MHz");
    }

    #[test]
    fn nice_ceiling_rounds_up_with_headroom() {
        assert_eq!(nice_ceiling(0.0), 1024.0);
        assert_eq!(nice_ceiling(1500.0), 2048.0);
        assert_eq!(nice_ceiling(4000.0), 5.0 * 1024.0);
        assert_eq!(nice_ceiling(3.0 * 1024.0 * 1024.0), 5.0 * 1024.0 * 1024.0);
        assert!(nice_ceiling(123_456.0) >= 123_456.0 * 1.15);
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(250 * 1024 * 1024), "250 MB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn formats_rate_and_percent() {
        assert_eq!(rate(0), "—");
        assert_eq!(rate(2048), "2.0 KB/s");
        assert_eq!(percent(0.01), "0%");
        assert_eq!(percent(3.14), "3.1%");
        assert_eq!(percent(42.6), "43%");
    }
}
