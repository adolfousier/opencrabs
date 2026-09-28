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
        let Some((key, val)) = normalize_declaration(line) else {
            continue;
        };

        if !is_tz_key(&key) {
            continue;
        }

        if let Some(info) = parse_tz_value(&val) {
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
                    // A line reaching this arm resolved NOWHERE else, so the
                    // parenthetical is the only declared label there is. Returning
                    // `None` here is what rendered `Europe/Moscow (МСК)` as a bare
                    // `Europe/Moscow` — carry it instead.
                    return Some(TzInfo::new(tz, parenthetical_label(trimmed)));
                }
            }
        }
    }

    None
}

/// Declared label carried by the parenthetical group of a free-form line.
///
/// The fallback scan matches a zone anywhere in a line, by which point the
/// declaration key is unknown — so the parenthetical is the only label the line
/// still offers. Returns `None` when the group is empty or is itself a zone
/// form (`Europe/Moscow (UTC+3)` declares no label, it declares an offset), so
/// a zone can never be mistaken for the user's own word.
fn parenthetical_label(line: &str) -> Option<String> {
    let (_, rest) = line.split_once('(')?;
    let inner = rest
        .trim_end_matches([')', '|', ' ', '\t'])
        .trim();
    if inner.is_empty() || inner.parse::<Tz>().is_ok() || parse_utc_offset_or_iana(inner).is_some() {
        return None;
    }
    Some(inner.to_string())
}

/// Normalise a `USER.md` declaration line into a `(key, value)` pair.
///
/// Handles the plain form (`Key: value`), markdown table rows
/// (`| Key | value |`), list markers (`- Key: value`), and bold/backtick/quote
/// wrappers around either side. Returns `None` when the line carries no
/// separator, so callers skip it without guessing.
///
/// Order matters: the table frame is stripped BEFORE the split. Splitting a row
/// like `| **Timezone** | UTC+3 (MSK) |` on its leading pipe yields an empty key
/// — that ordering is the defect this function exists to avoid.
fn normalize_declaration(line: &str) -> Option<(String, String)> {
    let mut s = line.trim();

    // Strip a markdown table frame: one leading and one trailing pipe.
    if let Some(rest) = s.strip_prefix('|') {
        s = rest.trim();
    }
    if let Some(rest) = s.strip_suffix('|') {
        s = rest.trim();
    }

    // Strip list markers (`- `, `* `, `# `) and blockquote markers.
    s = s.trim_start_matches(['-', '*', '#', '>', ' ']).trim();

    // Split on the FIRST `:` or `|`, whichever comes first.
    let sep = s.find([':', '|'])?;
    let key = unwrap_declaration_wrappers(&s[..sep]);
    let value = unwrap_declaration_wrappers(&s[sep + 1..]);

    if key.is_empty() || value.is_empty() {
        return None;
    }

    Some((key, value))
}

/// Strip emphasis/code/quote wrappers and surrounding whitespace from one side
/// of a declaration, so `**Timezone**` and `Timezone` compare equal.
fn unwrap_declaration_wrappers(s: &str) -> String {
    s.trim()
        .trim_matches(|c| {
            c == '*' || c == '_' || c == '`' || c == '"' || c == '\'' || c == '|'
        })
        .trim()
        .to_string()
}

/// Canonical `USER.md` keys that declare a timezone.
///
/// Matched case-insensitively against a key already normalised by
/// [`normalize_declaration`], so `Timezone`, `**Timezone**`, `TimeZone` and
/// `Часовой пояс` all resolve to the same declaration.
const TZ_KEYS: [&str; 3] = ["timezone", "time zone", "часовой пояс"];

/// Whether a normalised declaration key declares a timezone.
fn is_tz_key(key: &str) -> bool {
    let lower = key.trim().to_lowercase();
    TZ_KEYS.contains(&lower.as_str())
}

/// City-name and abbreviation aliases that resolve to a zone.
///
/// Deliberately narrow: the Moscow family only, per the approved design (DP1).
/// A wider Russian-city set is a follow-up owner decision, not an assumption —
/// every entry is a claim the parser acts on, so an unreviewed list would
/// silently resolve user text to the wrong zone.
const TZ_ALIASES: [(&str, Tz); 4] = [
    ("мск", Tz::Europe__Moscow),
    ("msk", Tz::Europe__Moscow),
    ("москва", Tz::Europe__Moscow),
    ("moscow", Tz::Europe__Moscow),
];

/// Resolve a city name or abbreviation to a zone.
///
/// Case-insensitive; returns `None` for anything outside [`TZ_ALIASES`].
fn tz_alias_lookup(s: &str) -> Option<Tz> {
    let lower = s.trim().to_lowercase();
    TZ_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lower)
        .map(|(_, tz)| *tz)
}

/// Parse a timezone value string into TzInfo.
fn parse_tz_value(val: &str) -> Option<TzInfo> {
    let val = val.trim();

    if let Some((outer, inner)) = split_parenthetical(val) {
        // Precedence is regression-critical. An EXPLICIT zone form — IANA name
        // or UTC offset — wins its slot over a city alias, in either position,
        // so `Москва (UTC+3)` keeps resolving to the offset with the label
        // "Москва" (pinned by parse_russian_timezone_format). Aliases are only
        // consulted once BOTH explicit slots are ruled out, which is what makes
        // `Москва (МСК)` resolve without disturbing the offset form.
        if let Some(tz) = parse_utc_offset_or_iana(outer) {
            return Some(TzInfo::new(tz, label_of(inner)));
        }
        if let Some(tz) = parse_utc_offset_or_iana(inner) {
            return Some(TzInfo::new(tz, label_of(outer)));
        }
        if let Some(tz) = tz_alias_lookup(outer) {
            return Some(TzInfo::new(tz, label_of(inner)));
        }
        if let Some(tz) = tz_alias_lookup(inner) {
            return Some(TzInfo::new(tz, label_of(outer)));
        }
    }

    // No parenthetical, or nothing in it resolved.
    // Direct IANA parse, e.g. "Europe/Paris"
    if let Ok(tz) = val.parse::<Tz>() {
        return Some(TzInfo::new(tz, None));
    }

    // Direct UTC offset parse, e.g. "UTC+3" or "+03:00"
    if let Some(tz) = parse_utc_offset_or_iana(val) {
        return Some(TzInfo::new(tz, None));
    }

    // Bare city name or abbreviation, e.g. "Москва" or "MSK". The alias is its
    // own label: the user's own word is what should appear in the marker.
    if let Some(tz) = tz_alias_lookup(val) {
        return Some(TzInfo::new(tz, label_of(val)));
    }

    None
}

/// Split `outer (inner)` into its two trimmed parts.
fn split_parenthetical(val: &str) -> Option<(&str, &str)> {
    let (outer, rest) = val.split_once('(')?;
    Some((outer.trim(), rest.trim_end_matches(')').trim()))
}

/// Label taken from one declaration slot, or `None` when the slot is empty.
fn label_of(slot: &str) -> Option<String> {
    let slot = slot.trim();
    if slot.is_empty() {
        None
    } else {
        Some(slot.to_string())
    }
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

    if !(0..=14).contains(&hours) {
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
            if let Some(ref entry) = *guard
                && entry.mtime == current_mtime
                && entry.file_len == current_len
            {
                return entry.resolved.clone();
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
