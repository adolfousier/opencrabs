use std::time::Duration;
use tokio::process::Command;

/// Condition required for a trigger to fire.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TriggerCondition {
    /// Fires if stdout or stderr contains any non-whitespace output (default).
    #[default]
    NonEmpty,
    /// Fires if the command exits with a non-zero exit code.
    ExitNonZero,
    /// Fires if the command exits with a zero exit code (success).
    ExitZero,
    /// Fires if the combined stdout/stderr output matches the provided regular expression.
    Regex(String),
    /// Fires whenever the trigger command finishes executing, regardless of output or exit status.
    Always,
}

impl TriggerCondition {
    pub fn parse(s: Option<&str>) -> Self {
        let raw = match s.map(str::trim) {
            Some(v) if !v.is_empty() => v,
            _ => return Self::NonEmpty,
        };

        if let Some(pat) = raw
            .strip_prefix("regex:")
            .or_else(|| raw.strip_prefix("re:"))
        {
            return Self::Regex(pat.trim().to_string());
        }

        match raw.to_lowercase().as_str() {
            "exit_non_zero" | "exitnonzero" | "non_zero" => Self::ExitNonZero,
            "exit_zero" | "exitzero" | "zero" | "exit_0" => Self::ExitZero,
            "always" => Self::Always,
            _ => Self::NonEmpty,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::NonEmpty => "non_empty",
            Self::ExitNonZero => "exit_non_zero",
            Self::ExitZero => "exit_zero",
            Self::Regex(s) => s.as_str(),
            Self::Always => "always",
        }
    }

    pub fn should_fire(&self, result: &TriggerResult) -> bool {
        match self {
            Self::NonEmpty => !result.combined_output().trim().is_empty(),
            Self::ExitNonZero => result.exit_code != 0,
            Self::ExitZero => result.exit_code == 0,
            Self::Regex(pat) => match regex::Regex::new(pat) {
                Ok(re) => re.is_match(&result.combined_output()),
                Err(e) => {
                    tracing::warn!("Invalid regex trigger condition '{pat}': {e}");
                    false
                }
            },
            Self::Always => true,
        }
    }
}

/// Result of executing a pre-flight trigger command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl TriggerResult {
    pub fn combined_output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else if self.stdout.is_empty() {
            self.stderr.clone()
        } else {
            format!("{}\n{}", self.stdout, self.stderr)
        }
    }
}

/// Runner for pre-flight trigger shell commands.
#[derive(Debug, Clone)]
pub struct TriggerRunner {
    timeout: Duration,
}

impl Default for TriggerRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
        }
    }
}

impl TriggerRunner {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Run the given shell command under `/bin/sh -c` with the configured timeout.
    pub async fn run(&self, cmd: &str) -> Result<TriggerResult, String> {
        let child = Command::new("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn trigger process: {e}"))?;

        let output_res = tokio::time::timeout(self.timeout, child.wait_with_output()).await;

        match output_res {
            Ok(Ok(output)) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Ok(TriggerResult {
                    exit_code,
                    stdout,
                    stderr,
                })
            }
            Ok(Err(e)) => Err(format!("Trigger process execution error: {e}")),
            Err(_) => Err(format!(
                "Trigger command timed out after {}s",
                self.timeout.as_secs()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_condition_parsing() {
        assert_eq!(TriggerCondition::parse(None), TriggerCondition::NonEmpty);
        assert_eq!(
            TriggerCondition::parse(Some("non_empty")),
            TriggerCondition::NonEmpty
        );
        assert_eq!(
            TriggerCondition::parse(Some("exit_non_zero")),
            TriggerCondition::ExitNonZero
        );
        assert_eq!(
            TriggerCondition::parse(Some("exit_zero")),
            TriggerCondition::ExitZero
        );
        assert_eq!(
            TriggerCondition::parse(Some("exitzero")),
            TriggerCondition::ExitZero
        );
        assert_eq!(
            TriggerCondition::parse(Some("regex:disk [0-9]+%")),
            TriggerCondition::Regex("disk [0-9]+%".into())
        );
        assert_eq!(
            TriggerCondition::parse(Some("re:ERROR.*")),
            TriggerCondition::Regex("ERROR.*".into())
        );
        assert_eq!(
            TriggerCondition::parse(Some("always")),
            TriggerCondition::Always
        );
    }

    #[test]
    fn test_trigger_condition_should_fire() {
        let empty_success = TriggerResult {
            exit_code: 0,
            stdout: "".into(),
            stderr: "".into(),
        };
        let with_output_success = TriggerResult {
            exit_code: 0,
            stdout: "disk 95%\n".into(),
            stderr: "".into(),
        };
        let empty_fail = TriggerResult {
            exit_code: 1,
            stdout: "".into(),
            stderr: "".into(),
        };

        // NonEmpty
        assert!(!TriggerCondition::NonEmpty.should_fire(&empty_success));
        assert!(TriggerCondition::NonEmpty.should_fire(&with_output_success));
        assert!(!TriggerCondition::NonEmpty.should_fire(&empty_fail));

        // ExitNonZero
        assert!(!TriggerCondition::ExitNonZero.should_fire(&empty_success));
        assert!(!TriggerCondition::ExitNonZero.should_fire(&with_output_success));
        assert!(TriggerCondition::ExitNonZero.should_fire(&empty_fail));

        // ExitZero
        assert!(TriggerCondition::ExitZero.should_fire(&empty_success));
        assert!(TriggerCondition::ExitZero.should_fire(&with_output_success));
        assert!(!TriggerCondition::ExitZero.should_fire(&empty_fail));

        // Regex
        let re_cond = TriggerCondition::Regex("disk [0-9]+%".into());
        assert!(!re_cond.should_fire(&empty_success));
        assert!(re_cond.should_fire(&with_output_success));
        assert!(!re_cond.should_fire(&empty_fail));

        // Always
        assert!(TriggerCondition::Always.should_fire(&empty_success));
        assert!(TriggerCondition::Always.should_fire(&with_output_success));
        assert!(TriggerCondition::Always.should_fire(&empty_fail));
    }

    #[tokio::test]
    async fn test_trigger_runner_execution() {
        let runner = TriggerRunner::default();
        let res = runner.run("echo 'hello trigger'").await.unwrap();
        assert_eq!(res.exit_code, 0);
        assert_eq!(res.stdout.trim(), "hello trigger");
        assert!(TriggerCondition::NonEmpty.should_fire(&res));

        let res_fail = runner.run("echo 'err' >&2; exit 2").await.unwrap();
        assert_eq!(res_fail.exit_code, 2);
        assert_eq!(res_fail.stderr.trim(), "err");
        assert!(TriggerCondition::ExitNonZero.should_fire(&res_fail));
    }
}
