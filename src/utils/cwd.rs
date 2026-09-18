//! Launch working-directory sanitization.
//!
//! A process started by a Windows service, scheduled task, registry Run
//! key, or a shortcut with no explicit "Start in" directory begins in a
//! SYSTEM directory — typically `C:\Windows\System32`. That is almost
//! never where an agent should root its work: every relative tool path
//! then resolves inside the system directory, `write_file` refuses
//! home-directory targets as "outside the working directory", shell
//! commands litter artifacts into the system tree, and the Runtime Info
//! tells the model its project is `C:\Windows\system32`.
//!
//! [`launch_cwd`] is the single decision point: use the process working
//! directory unless it is a Windows system directory, in which case fall
//! back to the user's home directory. Every startup site that seeds a
//! working directory from `std::env::current_dir()` should call this
//! instead. Explicit user choices (launching from a real project
//! directory, `/cd`) are never touched — only the system-directory
//! accident is corrected.

use std::path::{Path, PathBuf};

/// The directory the process should treat as its working directory at
/// startup: the process cwd, unless that is a Windows system directory
/// (service/scheduled-task launch), in which case the user's home.
pub fn launch_cwd() -> PathBuf {
    sanitize_launch_cwd(std::env::current_dir().unwrap_or_default())
}

/// Pure form of [`launch_cwd`]: map `raw` to the directory to use, so the
/// decision is testable without touching the process environment.
pub fn sanitize_launch_cwd(raw: PathBuf) -> PathBuf {
    if is_windows_system_dir(&raw) {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    raw
}

/// Is `p` one of the Windows system directories a service/task launch
/// lands in (`%SystemRoot%`, `System32`, `SysWOW64`, `Sysnative`)?
/// Case- and separator-insensitive; always false off Windows.
#[cfg(windows)]
fn is_windows_system_dir(p: &Path) -> bool {
    fn norm(p: &Path) -> String {
        p.to_string_lossy().replace('/', "\\").to_ascii_lowercase()
    }
    let raw = norm(p);
    let raw = raw.trim_end_matches('\\');

    let root = std::env::var("SYSTEMROOT")
        .map(|v| v.replace('/', "\\").to_ascii_lowercase())
        .unwrap_or_else(|_| "c:\\windows".to_string());
    let root = root.trim_end_matches('\\').to_string();

    [
        root.clone(),
        format!("{root}\\system32"),
        format!("{root}\\syswow64"),
        format!("{root}\\sysnative"),
    ]
    .iter()
    .any(|c| raw == c.trim_end_matches('\\'))
}

#[cfg(not(windows))]
fn is_windows_system_dir(_p: &Path) -> bool {
    false
}
