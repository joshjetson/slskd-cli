//! Human-readable formatting helpers shared by the CLI and the TUI.

pub fn bytes(n: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n}B")
    } else {
        format!("{v:.1}{}", UNITS[u])
    }
}

pub fn speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec <= 0.0 {
        return "—".into();
    }
    format!("{:.0}KB/s", bytes_per_sec / 1024.0)
}

pub fn duration(secs: Option<i64>) -> String {
    match secs {
        Some(s) if s > 0 => format!("{}:{:02}", s / 60, s % 60),
        _ => "—".into(),
    }
}

pub fn bitrate(br: Option<i64>) -> String {
    match br {
        Some(b) if b > 0 => format!("{b}k"),
        _ => "—".into(),
    }
}

/// Truncate to `width` display columns, ending with `…` when shortened.
pub fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    if width <= 1 {
        return "…".into();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// Compress a slskd transfer state for narrow columns.
pub fn state(s: &str) -> &str {
    match s {
        "Completed, Succeeded" => "done",
        "Completed, Errored" => "error",
        "Completed, Cancelled" => "cancelled",
        "Completed, TimedOut" => "timeout",
        "InProgress" => "downloading",
        "Requested" => "requested",
        "Queued, Remotely" => "queued (peer)",
        "Queued, Locally" => "queued",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_human_sized() {
        assert_eq!(bytes(512), "512B");
        assert_eq!(bytes(8_358_105), "8.0MB");
    }

    #[test]
    fn duration_handles_missing_and_zero() {
        assert_eq!(duration(Some(206)), "3:26");
        assert_eq!(duration(Some(0)), "—");
        assert_eq!(duration(None), "—");
    }

    #[test]
    fn truncate_is_width_exact() {
        assert_eq!(truncate("abcdef", 10), "abcdef");
        assert_eq!(truncate("abcdef", 4).chars().count(), 4);
        assert_eq!(truncate("abcdef", 4), "abc…");
    }
}
