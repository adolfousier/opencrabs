//! String utility functions.

/// Truncate a string to at most `max_bytes` bytes, ensuring the cut lands on a
/// valid UTF-8 char boundary. Returns the longest prefix that fits.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Collapse the current user's `$HOME` prefix to `~` in paths/commands.
/// `/Users/alice/srv/foo/bar.rs` → `~/srv/foo/bar.rs`. Keeps absolute
/// paths OUTSIDE home untouched so `/tmp/...` or `/etc/...` still render
/// faithfully. No-op if home isn't resolvable or doesn't appear in `s`.
pub fn tilde_home(s: &str) -> String {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return s.to_string(),
    };
    let home_str = home.to_string_lossy();
    if home_str.is_empty() {
        return s.to_string();
    }
    // Replace all occurrences — bash commands like `cd /Users/me/a && cp /Users/me/b /Users/me/c`
    // benefit from every instance being collapsed.
    s.replace(home_str.as_ref(), "~")
}

/// Shorten a string to fit `max_bytes` while preserving both ends —
/// essential for file paths where the filename (tail) is usually the
/// most informative part. `~/a/b/c/d/very_long_name.rs` truncated to
/// 30 bytes becomes `~/a/b/…/very_long_name.rs` rather than
/// `~/a/b/c/d/very_long_na` which loses the `.rs` extension entirely.
///
/// Respects UTF-8 char boundaries on both sides of the ellipsis.
pub fn truncate_middle(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    const ELLIPSIS: &str = "…"; // 3 bytes
    if max_bytes <= ELLIPSIS.len() + 2 {
        // Too small to preserve both ends meaningfully — fall back to head truncation.
        return truncate_str(s, max_bytes).to_string();
    }
    let budget = max_bytes - ELLIPSIS.len();
    // Slight bias toward keeping the tail since filenames / final args
    // carry more signal than the leading path components.
    let tail_bytes = budget.div_ceil(2);
    let head_bytes = budget - tail_bytes;

    let mut head_end = head_bytes;
    while head_end > 0 && !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = s.len() - tail_bytes;
    while tail_start < s.len() && !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    if tail_start <= head_end {
        return truncate_str(s, max_bytes).to_string();
    }
    format!("{}{}{}", &s[..head_end], ELLIPSIS, &s[tail_start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_str_ascii() {
        assert_eq!(truncate_str("hello world", 5), "hello");
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_str_multibyte_boundary() {
        // █ is U+2588, 3 bytes in UTF-8
        let s = "abc█def";
        // "abc" = 3 bytes, "█" = bytes 3..6, "def" = bytes 6..9
        assert_eq!(truncate_str(s, 3), "abc"); // exact boundary before █
        assert_eq!(truncate_str(s, 4), "abc"); // inside █, backs up to 3
        assert_eq!(truncate_str(s, 5), "abc"); // inside █, backs up to 3
        assert_eq!(truncate_str(s, 6), "abc█"); // exact boundary after █
    }

    #[test]
    fn test_truncate_str_emoji() {
        // 🦀 is U+1F980, 4 bytes in UTF-8
        let s = "hi🦀bye";
        // "hi" = 2 bytes, "🦀" = bytes 2..6, "bye" = bytes 6..9
        assert_eq!(truncate_str(s, 2), "hi");
        assert_eq!(truncate_str(s, 3), "hi"); // inside 🦀
        assert_eq!(truncate_str(s, 5), "hi"); // inside 🦀
        assert_eq!(truncate_str(s, 6), "hi🦀");
    }

    #[test]
    fn test_truncate_str_zero() {
        assert_eq!(truncate_str("hello", 0), "");
        assert_eq!(truncate_str("🦀", 0), "");
    }

    #[test]
    fn test_truncate_str_empty() {
        assert_eq!(truncate_str("", 5), "");
        assert_eq!(truncate_str("", 0), "");
    }

    #[test]
    fn test_truncate_str_all_multibyte() {
        // Each char is 3 bytes
        let s = "███"; // 9 bytes
        assert_eq!(truncate_str(s, 1), ""); // inside first █
        assert_eq!(truncate_str(s, 3), "█");
        assert_eq!(truncate_str(s, 7), "██"); // inside third █
        assert_eq!(truncate_str(s, 9), "███");
    }

    #[test]
    fn test_truncate_middle_preserves_tail() {
        let s = "/Users/alice/srv/dart/heyiolo/lib/presentation/pages/some_really_long_widget_name.dart";
        let out = truncate_middle(s, 40);
        // The filename must survive — that's the whole point of middle-elide.
        assert!(
            out.ends_with("some_really_long_widget_name.dart")
                || out.ends_with("_name.dart")
                || out.ends_with(".dart"),
            "expected filename tail preserved, got: {}",
            out
        );
        assert!(out.contains('…'), "expected ellipsis marker in middle");
        assert!(out.len() <= 40, "expected <= 40 bytes, got {}", out.len());
    }

    #[test]
    fn test_truncate_middle_short_string_unchanged() {
        assert_eq!(truncate_middle("short.rs", 80), "short.rs");
    }

    #[test]
    fn test_truncate_middle_tiny_budget_falls_back() {
        let out = truncate_middle("hello world", 4);
        // Budget too small for head+…+tail — falls back to head-only truncation.
        assert!(out.len() <= 4);
    }

    #[test]
    fn test_tilde_home_collapses_prefix() {
        if let Some(h) = dirs::home_dir() {
            let home_str = h.to_string_lossy();
            let full = format!("{}/srv/project/file.rs", home_str);
            assert_eq!(tilde_home(&full), "~/srv/project/file.rs");
            // Non-home paths untouched.
            assert_eq!(tilde_home("/tmp/foo"), "/tmp/foo");
            // Bash commands with multiple occurrences all collapsed.
            let cmd = format!("cp {}/a {}/b", home_str, home_str);
            assert_eq!(tilde_home(&cmd), "cp ~/a ~/b");
        }
    }
}
