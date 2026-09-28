//! Tests for user timezone heuristic parsing and caching.
//!
//! Extracted from an inline `#[cfg(test)]` block in
//! `src/brain/timezone.rs`; project policy (CONTRIBUTING.md) requires
//! all tests under `src/tests/`.

use std::fs;
use std::io::Write;

use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use tempfile::NamedTempFile;

use crate::brain::timezone::{UserTimezoneCache, parse_timezone_heuristic};

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

/// A line whose declaration key is NOT a timezone key, and which carries no
/// separator at all, resolves only through the secondary IANA scan. That arm
/// must carry the declared parenthetical through: before task 5 it returned a
/// bare zone, so the user's own label was silently dropped.
#[test]
fn fallback_scan_carries_declared_label() {
    let user_md = "\
# Notes
Working hours — Europe/Moscow (МСК)
";
    let info = parse_timezone_heuristic(user_md).expect("fallback scan should resolve");
    assert_eq!(info.tz, Tz::Europe__Moscow);
    assert_eq!(info.label.as_deref(), Some("МСК"));
}

/// The parenthetical is a label only when it is NOT itself a zone form:
/// `(UTC+3)` declares an offset, not a name, so it must never become the label.
#[test]
fn fallback_scan_ignores_zone_form_parenthetical() {
    let user_md = "\
# Notes
Standup — Europe/Moscow (UTC+3)
";
    let info = parse_timezone_heuristic(user_md).expect("fallback scan should resolve");
    assert_eq!(info.tz, Tz::Europe__Moscow);
    assert_eq!(info.label, None);
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
    writeln!(tmp, "Timezone: UTC+2 (EET)").unwrap();
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
    writeln!(file, "Timezone: UTC+7 (NOVT)").unwrap();
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

#[test]
fn prose_mention_of_timezone_mid_sentence_does_not_match() {
    // "timezone" appears here with a colon, but NOT in the key position. The
    // matcher compares the whole normalised key, never a substring, so this
    // line must not hijack the declaration.
    let user_md = "\
# Notes
My timezone: is a personal matter, ask me instead.
";
    assert!(parse_timezone_heuristic(user_md).is_none());
}

#[test]
fn parse_moscow_city_with_abbreviation_label() {
    // Alias in the OUTER slot: the explicit-zone arms must be ruled out first,
    // then the alias resolves and the parenthetical becomes the label.
    let user_md = "\
# Профиль
- **Часовой пояс:** Москва (МСК)
";
    let info = parse_timezone_heuristic(user_md).expect("should parse");
    assert_eq!(info.tz, Tz::Europe__Moscow);
    assert_eq!(info.label.as_deref(), Some("МСК"));
}

#[test]
fn parse_bare_moscow_abbreviation() {
    let user_md = "\
# Профиль
**Часовой пояс:** МСК
";
    let info = parse_timezone_heuristic(user_md).expect("should parse");
    assert_eq!(info.tz, Tz::Europe__Moscow);
    assert_eq!(info.label.as_deref(), Some("МСК"));
}
