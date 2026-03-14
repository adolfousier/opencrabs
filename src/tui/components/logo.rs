/// ASCII art logo for OpenCrabs featuring a croissant
pub const LOGO: &str = r#"
   ╔═══════════════════════════════════════════════════════════════╗
   ║                                                               ║
   ║         ██████╗██████╗ ██╗   ██╗███████╗████████╗██╗   ██╗   ║
   ║        ██╔════╝██╔══██╗██║   ██║██╔════╝╚══██╔══╝██║   ██║   ║
   ║        ██║     ██████╔╝██║   ██║███████╗   ██║   ██║   ██║   ║
   ║        ██║     ██╔══██╗██║   ██║╚════██║   ██║   ██║   ██║   ║
   ║        ╚██████╗██║  ██║╚██████╔╝███████║   ██║   ███████║╚██╗ ║
   ║         ╚═════╝╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝   ╚══════╝ ╚═╝ ║
   ║                                                               ║
   ║                    ,r-~~-,                                    ║
   ║                 ,-'        `~~-,                              ║
   ║              ,-'      ,r~~-,    `~-,                          ║
   ║           ,-'      ,-'      `-,     `-,                       ║
   ║        ,-'      ,-'     🥐     `-,     `-,                    ║
   ║     ,-'      ,-'                  `-,     `-,                 ║
   ║   ,'      ,-'                       `-,     `~,               ║
   ║  (      ,'                             `-,    \               ║
   ║   `-_, '                                  `-,_/               ║
   ║                                                               ║
   ║ The autonomous, self-improving AI agent. Single binary. Every channel. ║
   ║                                                               ║
   ╚═══════════════════════════════════════════════════════════════╝
"#;

pub const CROISSANT: &str = r#"
              ,r-~~-,
           ,-'        `~~-,
        ,-'      ,r~~-,    `~-,
     ,-'      ,-'      `-,     `-,
  ,-'      ,-'     🥐     `-,     `-,
,'      ,-'                  `-,     `-,
      ,'                       `-,     `~,
    ,'                             `-,    \
   '                                  `-,_/
"#;

pub const SMALL_LOGO: &str = r#"
   ___             _   _
  / __|_ _ _  _ __| |_| |_  _
 | (__| '_| || (_-<  _| | || |
  \___|_|  \_,_/__/\__|_|\_, |
                         |__/
        🥐 Flaky & Fast
"#;

/// Returns the full OpenCrabs logo with croissant
pub fn get_logo() -> &'static str {
    LOGO
}

/// Returns just the croissant ASCII art
pub fn get_croissant() -> &'static str {
    CROISSANT
}

/// Returns a smaller version of the logo
pub fn get_small_logo() -> &'static str {
    SMALL_LOGO
}

/// Returns the logo with version information
pub fn get_logo_with_version(version: &str) -> String {
    format!("{}\n   Version: {}\n", LOGO, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logo_not_empty() {
        assert!(!get_logo().is_empty());
        assert!(!get_croissant().is_empty());
        assert!(!get_small_logo().is_empty());
    }

    #[test]
    fn test_logo_with_version() {
        let logo = get_logo_with_version("0.1.0");
        assert!(logo.contains("0.1.0"));
    }
}
