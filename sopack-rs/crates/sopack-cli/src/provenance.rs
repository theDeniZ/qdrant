//! Default `pack_id` / `created_by` generation — mirrors
//! `sopack.pack._default_pack_id` / `_default_created_by` exactly enough to
//! produce the same shape (`"<profile>-<YYYY-MM-DD>-<4 hex chars>"`,
//! `"sopack <version> (rust) on <os> <release> <arch>"`), without a chrono
//! dependency: this workspace pins only `ort`/`tokenizers`/`ndarray` at the
//! shared level (`common.md`'s "add an explicit version in your own
//! crate's Cargo.toml" is for a *new* dependency; a hand-rolled UTC date
//! avoids needing one at all here).

use std::time::{SystemTime, UNIX_EPOCH};

/// Civil (Y-M-D) date from a Unix day count, Howard Hinnant's
/// `civil_from_days` algorithm (proleptic Gregorian, valid for every date
/// this process will ever see). Avoids a chrono/time dependency for one
/// `YYYY-MM-DD` stamp.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Today's UTC date as `YYYY-MM-DD`.
pub fn today_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// 4 hex characters (2 pseudo-random bytes) to disambiguate same-day
/// `pack_id`s — not cryptographic, just a collision-avoidance suffix, so
/// process time + pid + a nanosecond counter is plenty rather than pulling
/// in a `rand`/extra `uuid` feature for it.
fn random_hex4() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let pid = std::process::id();
    let mixed = nanos ^ pid.wrapping_mul(2_654_435_761);
    format!("{:04x}", (mixed & 0xffff) as u16)
}

/// `"<profile>-<YYYY-MM-DD>-<hex4>"` — `sopack.pack._default_pack_id`.
pub fn default_pack_id(profile_name: &str) -> String {
    format!("{profile_name}-{}-{}", today_utc(), random_hex4())
}

/// `"sopack <version> (rust) on <os> <release> <arch>"` —
/// `sopack.pack._default_created_by`. `<release>` comes from `uname -r`
/// (best-effort; `"unknown"` if that fails), matching Python's
/// `platform.release()` since Rust's stdlib has no portable equivalent.
pub fn default_created_by() -> String {
    let os = os_display_name();
    let release = uname_release().unwrap_or_else(|| "unknown".to_string());
    let arch = std::env::consts::ARCH;
    format!(
        "sopack {} (rust) on {os} {release} {arch}",
        env!("CARGO_PKG_VERSION")
    )
}

fn os_display_name() -> &'static str {
    match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "Darwin",
        other => other,
    }
}

fn uname_release() -> Option<String> {
    let out = std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_epoch_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-09-24 is 20720 days after the epoch.
        assert_eq!(civil_from_days(20720), (2026, 9, 24));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn today_utc_has_the_right_shape() {
        let s = today_utc();
        assert_eq!(s.len(), 10);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert!(s.chars().all(|c| c.is_ascii_digit() || c == '-'));
    }

    #[test]
    fn default_pack_id_has_the_expected_shape() {
        let id = default_pack_id("sop");
        let parts: Vec<&str> = id.split('-').collect();
        // "sop" - "2026" - "09" - "24" - "<hex4>"
        assert_eq!(parts[0], "sop");
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[4].len(), 4);
        assert!(parts[4].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn default_created_by_names_this_binary_and_the_platform() {
        let s = default_created_by();
        assert!(s.starts_with("sopack "));
        assert!(s.contains("(rust) on"));
        assert!(s.contains(std::env::consts::ARCH));
    }

    #[test]
    fn random_hex4_is_four_lowercase_hex_chars() {
        let h = random_hex4();
        assert_eq!(h.len(), 4);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
