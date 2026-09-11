//! User timezone resolution and caching (#153).
//!
//! Provides temporal grounding for agents by parsing user-preferred timezones
//! from `USER.md` (e.g. `Timezone: UTC+3 (MSK)` or `Europe/Paris`) and caching
//! the resolved [`chrono_tz::Tz`]. Keyed on `USER.md` mtime and content hash
//! to avoid disk/LLM overhead on subsequent turns while invalidating automatically
//! when `USER.md` is modified.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use once_cell::sync::Lazy;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

/// Resolved timezone info with display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TzInfo {
    pub tz: Tz,
    pub label: Option<String>,
}

impl TzInfo {
    pub fn new(tz: Tz, label: Option<String>) -> Self {
        Self { tz, label }
    }

    /// Format a UTC timestamp into dual UTC + local representation:
    /// `YYYY-MM-DD HH:MM:SS UTC (user: HH:MM:SS TZ)`
    pub fn format_dual_time(&self, dt: &DateTime<Utc>) -> String {
        let utc_str = dt.format("%Y-%m-%d %H:%M:%S UTC");
        let local_dt = dt.with_timezone(&self.tz);
        let tz_label = self.label.as_deref().unwrap_or_else(|| self.tz.name());
        let local_str = local_dt.format("%H:%M:%S");
        format!("{utc_str} (user: {local_str} {tz_label})")
    }
}

/// Format UTC-only time marker string.
pub fn format_utc_time(dt: &DateTime<Utc>) -> String {
    format!("{} UTC", dt.format("%Y-%m-%d %H:%M:%S"))
}

/// Tier 1: Fast synchronous parser for timezone declarations in text (USER.md).
/// Matches common formats:
/// - `Timezone: UTC+3 (MSK)` / `Timezone: UTC-5 (EST)`
/// - `Timezone: Europe/Moscow`
/// - `Часовой пояс: UTC+3 (MSK)` / `Часовой пояс: Москва (UTC+3)`
/// - `Timezone: America/New_York`
pub fn parse_timezone_heuristic(text: &str) -> Option<TzInfo> {
    for line in text.lines() {
        let line_clean = line
            .trim()
            .trim_start_matches(|c| c == '-' || c == '*' || c == '#')
            .trim();
        let lower = line_clean.to_lowercase();

        let is_tz_line = lower.starts_with("timezone:")
            || lower.starts_with("timezone :")
            || lower.starts_with("часовой пояс:")
            || lower.starts_with("часовой пояс :");

        if !is_tz_line
            && !line_clean.starts_with("**Timezone:**")
            && !line_clean.starts_with("**Часовой пояс:**")
        {
            continue;
        }

        // Extract the value after colon
        let Some((_, val_part)) = line_clean.split_once(':') else {
            continue;
        };
        let val = val_part
            .trim()
            .trim_matches(|c| c == '*' || c == '`' || c == '"' || c == '\'')
            .trim();

        if let Some(info) = parse_tz_value(val) {
            return Some(info);
        }
    }

    // Secondary scan: check if any standalone line contains an explicit IANA or UTC offset pattern
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("Europe/") || trimmed.contains("America/") || trimmed.contains("Asia/")
        {
            for word in trimmed.split_whitespace() {
                let clean_word = word.trim_matches(|c| {
                    c == '(' || c == ')' || c == '[' || c == ']' || c == '`' || c == ',' || c == '.'
                });
                if let Ok(tz) = clean_word.parse::<Tz>() {
                    return Some(TzInfo::new(tz, None));
                }
            }
        }
    }

    None
}

/// Parse a timezone value string into TzInfo.
fn parse_tz_value(val: &str) -> Option<TzInfo> {
    // Check for "UTC+3 (MSK)" or "UTC-5 (EST)"
    if let Some((utc_part, rest)) = val.split_once('(') {
        let label = rest.trim_end_matches(')').trim().to_string();
        let tz_candidate = utc_part.trim();
        if let Some(tz) = parse_utc_offset_or_iana(tz_candidate) {
            return Some(TzInfo::new(
                tz,
                if label.is_empty() { None } else { Some(label) },
            ));
        }
        // Maybe format is "Москва (UTC+3)"
        let inner = rest.trim_end_matches(')').trim();
        if let Some(tz) = parse_utc_offset_or_iana(inner) {
            let outer_label = utc_part.trim().to_string();
            return Some(TzInfo::new(
                tz,
                if outer_label.is_empty() {
                    None
                } else {
                    Some(outer_label)
                },
            ));
        }
    }

    // Direct IANA parse, e.g. "Europe/Paris"
    if let Ok(tz) = val.parse::<Tz>() {
        return Some(TzInfo::new(tz, None));
    }

    // Direct UTC offset parse, e.g. "UTC+3" or "+03:00"
    if let Some(tz) = parse_utc_offset_or_iana(val) {
        return Some(TzInfo::new(tz, None));
    }

    None
}

/// Parse UTC offset (e.g. "UTC+3", "UTC-5", "UTC+03:00", "+03") or direct IANA string.
fn parse_utc_offset_or_iana(s: &str) -> Option<Tz> {
    let s = s.trim();
    if let Ok(tz) = s.parse::<Tz>() {
        return Some(tz);
    }

    let upper = s.to_uppercase();
    let offset_str = upper
        .strip_prefix("UTC")
        .or_else(|| upper.strip_prefix("GMT"))
        .unwrap_or(&upper)
        .trim();

    // Map common fixed offsets to Etc/GMT offsets.
    // Note: In POSIX/IANA Etc/GMT zones, the sign is inverted (Etc/GMT-3 is UTC+3).
    let sign_and_num = offset_str
        .strip_prefix('+')
        .map(|num| (1, num))
        .or_else(|| offset_str.strip_prefix('-').map(|num| (-1, num)))?;

    let hours_str = sign_and_num.1.split(':').next()?.trim();
    let hours: i32 = hours_str.parse().ok()?;

    if hours > 14 || hours < 0 {
        return None;
    }

    let total_offset = sign_and_num.0 * hours;
    etc_gmt_for_offset(total_offset)
}

/// Convert standard UTC hour offset (e.g. +3 for MSK) to chrono_tz::Tz.
fn etc_gmt_for_offset(offset: i32) -> Option<Tz> {
    // POSIX Etc/GMT signs are inverted: UTC+3 is Etc/GMT-3, UTC-5 is Etc/GMT+5.
    match offset {
        0 => Some(Tz::UTC),
        1 => Some(Tz::Etc__GMTMinus1),
        2 => Some(Tz::Etc__GMTMinus2),
        3 => Some(Tz::Etc__GMTMinus3),
        4 => Some(Tz::Etc__GMTMinus4),
        5 => Some(Tz::Etc__GMTMinus5),
        6 => Some(Tz::Etc__GMTMinus6),
        7 => Some(Tz::Etc__GMTMinus7),
        8 => Some(Tz::Etc__GMTMinus8),
        9 => Some(Tz::Etc__GMTMinus9),
        10 => Some(Tz::Etc__GMTMinus10),
        11 => Some(Tz::Etc__GMTMinus11),
        12 => Some(Tz::Etc__GMTMinus12),
        -1 => Some(Tz::Etc__GMTPlus1),
        -2 => Some(Tz::Etc__GMTPlus2),
        -3 => Some(Tz::Etc__GMTPlus3),
        -4 => Some(Tz::Etc__GMTPlus4),
        -5 => Some(Tz::Etc__GMTPlus5),
        -6 => Some(Tz::Etc__GMTPlus6),
        -7 => Some(Tz::Etc__GMTPlus7),
        -8 => Some(Tz::Etc__GMTPlus8),
        -9 => Some(Tz::Etc__GMTPlus9),
        -10 => Some(Tz::Etc__GMTPlus10),
        -11 => Some(Tz::Etc__GMTPlus11),
        -12 => Some(Tz::Etc__GMTPlus12),
        _ => None,
    }
}

/// In-memory cache holding resolved user timezone, keyed by file mtime and size.
#[derive(Debug, Default)]
pub struct UserTimezoneCache {
    inner: Mutex<Option<CachedEntry>>,
}

#[derive(Debug, Clone)]
struct CachedEntry {
    mtime: Option<SystemTime>,
    file_len: u64,
    resolved: Option<TzInfo>,
}

impl UserTimezoneCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Resolve timezone from `USER.md` in the given brain directory.
    /// If cached and `USER.md` mtime/size unchanged, returns cached without disk read.
    pub fn resolve_from_brain_dir(&self, brain_dir: &Path) -> Option<TzInfo> {
        let user_md_path = brain_dir.join("USER.md");
        self.resolve_from_file(&user_md_path)
    }

    /// Resolve timezone from a specific `USER.md` file path.
    pub fn resolve_from_file(&self, path: &Path) -> Option<TzInfo> {
        let meta = fs::metadata(path).ok();
        let current_mtime = meta.as_ref().and_then(|m| m.modified().ok());
        let current_len = meta.as_ref().map(|m| m.len()).unwrap_or(0);

        {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ref entry) = *guard {
                if entry.mtime == current_mtime && entry.file_len == current_len {
                    return entry.resolved.clone();
                }
            }
        }

        // Needs re-read or initial read
        let resolved = if let Ok(content) = fs::read_to_string(path) {
            parse_timezone_heuristic(&content)
        } else {
            None
        };

        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(CachedEntry {
            mtime: current_mtime,
            file_len: current_len,
            resolved: resolved.clone(),
        });

        resolved
    }

    /// Update or seed the cache explicitly (e.g. after Tier-2 LLM extraction).
    pub fn set_explicit(&self, path: &Path, resolved: Option<TzInfo>) {
        let meta = fs::metadata(path).ok();
        let current_mtime = meta.as_ref().and_then(|m| m.modified().ok());
        let current_len = meta.as_ref().map(|m| m.len()).unwrap_or(0);

        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(CachedEntry {
            mtime: current_mtime,
            file_len: current_len,
            resolved,
        });
    }

    /// Invalidate cache manually (e.g. on test teardown or explicit notification).
    pub fn invalidate(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }
}

/// Process-wide singleton cache for user timezone.
pub static GLOBAL_TZ_CACHE: Lazy<UserTimezoneCache> = Lazy::new(UserTimezoneCache::new);

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parse_standard_english_timezone() {
        let user_md = "\
# USER
- **Name:** Alexey
- **Timezone:** UTC+3 (MSK)
";
        let info = parse_timezone_heuristic(user_md).expect("should parse");
        assert_eq!(info.label.as_deref(), Some("MSK"));
        let dt = Utc.with_ymd_and_hms(2026, 9, 10, 12, 0, 0).unwrap();
        let formatted = info.format_dual_time(&dt);
        assert_eq!(formatted, "2026-09-10 12:00:00 UTC (user: 15:00:00 MSK)");
    }

    #[test]
    fn parse_russian_timezone_format() {
        let user_md = "\
# Профиль
- **Часовой пояс:** Москва (UTC+3)
";
        let info = parse_timezone_heuristic(user_md).expect("should parse");
        assert_eq!(info.label.as_deref(), Some("Москва"));
        let dt = Utc.with_ymd_and_hms(2026, 1, 15, 10, 0, 0).unwrap();
        let formatted = info.format_dual_time(&dt);
        assert_eq!(formatted, "2026-01-15 10:00:00 UTC (user: 13:00:00 Москва)");
    }

    #[test]
    fn parse_iana_timezone_with_dst_transitions() {
        let user_md = "\
# USER
Timezone: Europe/Paris
";
        let info = parse_timezone_heuristic(user_md).expect("should parse");
        assert_eq!(info.tz, Tz::Europe__Paris);

        // Summer: Paris is UTC+2 (CEST)
        let summer_dt = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
        let summer_fmt = info.format_dual_time(&summer_dt);
        assert_eq!(
            summer_fmt,
            "2026-07-01 12:00:00 UTC (user: 14:00:00 Europe/Paris)"
        );

        // Winter: Paris is UTC+1 (CET)
        let winter_dt = Utc.with_ymd_and_hms(2026, 12, 1, 12, 0, 0).unwrap();
        let winter_fmt = info.format_dual_time(&winter_dt);
        assert_eq!(
            winter_fmt,
            "2026-12-01 12:00:00 UTC (user: 13:00:00 Europe/Paris)"
        );
    }

    #[test]
    fn test_caching_and_invalidation() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "Timezone: UTC+2 (EET)\n").unwrap();
        tmp.flush().unwrap();

        let cache = UserTimezoneCache::new();
        let res1 = cache.resolve_from_file(tmp.path()).expect("first resolve");
        assert_eq!(res1.label.as_deref(), Some("EET"));

        // Rewrite file with new timezone
        std::thread::sleep(std::time::Duration::from_millis(15));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(tmp.path())
            .unwrap();
        write!(file, "Timezone: UTC+7 (NOVT)\n").unwrap();
        file.flush().unwrap();

        let res2 = cache.resolve_from_file(tmp.path()).expect("second resolve");
        assert_eq!(res2.label.as_deref(), Some("NOVT"));
    }

    #[test]
    fn test_negative_caching_on_no_match() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "# Just notes\nNo timezone mentioned here.\n").unwrap();
        tmp.flush().unwrap();

        let cache = UserTimezoneCache::new();
        assert!(cache.resolve_from_file(tmp.path()).is_none());

        // Second call hits cache returning None without re-reading
        assert!(cache.resolve_from_file(tmp.path()).is_none());
    }
}
