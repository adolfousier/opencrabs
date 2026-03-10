//! Install method detection.
//!
//! Determines how OpenCrabs was installed so that evolve, crash recovery,
//! and other update paths can use the correct upgrade strategy.

/// How the current binary was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMethod {
    /// Built from source — exe lives inside a cargo `target/` dir with a `Cargo.toml` ancestor.
    /// Contains the project root path.
    Source(std::path::PathBuf),
    /// Installed via `cargo install opencrabs` — exe lives in `~/.cargo/bin/`.
    CargoInstall,
    /// Pre-built binary downloaded from GitHub releases (or installed manually).
    PrebuiltBinary,
}

impl InstallMethod {
    /// Detect how the current binary was installed.
    pub fn detect() -> Self {
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(_) => return Self::PrebuiltBinary,
        };

        // Check if we're in a cargo target directory (source build)
        if let Some(project_root) = find_cargo_project(&exe) {
            return Self::Source(project_root);
        }

        // Check if we're in ~/.cargo/bin/ (cargo install)
        if is_in_cargo_bin(&exe) {
            return Self::CargoInstall;
        }

        Self::PrebuiltBinary
    }

    /// Human-readable description for UI display.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Source(_) => "source build",
            Self::CargoInstall => "cargo install",
            Self::PrebuiltBinary => "pre-built binary",
        }
    }
}

/// Walk up from the executable to find a Cargo.toml (indicating a source build).
fn find_cargo_project(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = exe.parent()?.to_path_buf();
    loop {
        if dir.join("Cargo.toml").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Check if the executable is in ~/.cargo/bin/.
fn is_in_cargo_bin(exe: &std::path::Path) -> bool {
    let cargo_bin = cargo_bin_dir();
    match cargo_bin {
        Some(dir) => exe.parent().map(|p| p == dir).unwrap_or(false),
        None => false,
    }
}

/// Get the cargo bin directory (~/.cargo/bin or $CARGO_HOME/bin).
fn cargo_bin_dir() -> Option<std::path::PathBuf> {
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        return Some(std::path::PathBuf::from(cargo_home).join("bin"));
    }
    dirs::home_dir().map(|h| h.join(".cargo").join("bin"))
}

/// Platform asset suffix for GitHub release downloads.
pub fn platform_suffix() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("macos", "x86_64") => Some("macos-amd64"),
        ("linux", "x86_64") => Some("linux-amd64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("windows", "x86_64") => Some("windows-amd64"),
        _ => None,
    }
}

/// Binary filename for the current platform.
pub fn binary_name() -> &'static str {
    if std::env::consts::OS == "windows" {
        "opencrabs.exe"
    } else {
        "opencrabs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_some_variant() {
        let method = InstallMethod::detect();
        // On CI or dev machines this should return Source (we're in a cargo project)
        // Just verify it doesn't panic and returns a valid variant
        let _ = method.description();
    }

    #[test]
    fn source_description() {
        let m = InstallMethod::Source(std::path::PathBuf::from("/tmp"));
        assert_eq!(m.description(), "source build");
    }

    #[test]
    fn cargo_install_description() {
        assert_eq!(InstallMethod::CargoInstall.description(), "cargo install");
    }

    #[test]
    fn prebuilt_description() {
        assert_eq!(
            InstallMethod::PrebuiltBinary.description(),
            "pre-built binary"
        );
    }

    #[test]
    fn platform_suffix_is_some_on_supported() {
        // On any standard dev machine this should return Some
        if matches!(
            (std::env::consts::OS, std::env::consts::ARCH),
            ("macos", "aarch64")
                | ("macos", "x86_64")
                | ("linux", "x86_64")
                | ("linux", "aarch64")
                | ("windows", "x86_64")
        ) {
            assert!(platform_suffix().is_some());
        }
    }

    #[test]
    fn binary_name_matches_platform() {
        let name = binary_name();
        if std::env::consts::OS == "windows" {
            assert_eq!(name, "opencrabs.exe");
        } else {
            assert_eq!(name, "opencrabs");
        }
    }
}
