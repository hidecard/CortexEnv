//! Runtime security-mode selection and fail-closed production gating.

use sandbox_common::SandboxError;

/// Deployment mode for runtime execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityMode {
    /// Development-only mode for trusted local code.
    Trusted,
    /// Production-oriented mode requiring the OS child supervisor.
    Restricted,
    /// Hostile-code mode requiring the complete OS isolation boundary.
    HostileCode,
}

impl SecurityMode {
    /// Parse the `CORTEXENV_SECURITY_MODE` value.
    pub fn parse(value: &str) -> Result<Self, SandboxError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "trusted" | "trusted-development" => Ok(Self::Trusted),
            "restricted" | "production" => Ok(Self::Restricted),
            "hostile-code" | "hostile" => Ok(Self::HostileCode),
            _ => Err(SandboxError::InvalidRequest(
                "CORTEXENV_SECURITY_MODE must be trusted, restricted, or hostile-code".into(),
            )),
        }
    }

    /// Read the configured mode, defaulting to trusted development mode.
    pub fn configured() -> Result<Self, SandboxError> {
        std::env::var("CORTEXENV_SECURITY_MODE")
            .map(|value| Self::parse(&value))
            .unwrap_or(Ok(Self::Trusted))
    }

    /// Reject in-process execution when the required supervisor is not active.
    pub fn ensure_in_process_execution_allowed(self) -> Result<(), SandboxError> {
        match self {
            Self::Trusted => Ok(()),
            Self::Restricted | Self::HostileCode => Err(SandboxError::Isolation),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_modes() {
        assert_eq!(
            SecurityMode::parse("trusted").unwrap(),
            SecurityMode::Trusted
        );
        assert_eq!(
            SecurityMode::parse("production").unwrap(),
            SecurityMode::Restricted
        );
        assert_eq!(
            SecurityMode::parse("HOSTILE-CODE").unwrap(),
            SecurityMode::HostileCode
        );
    }

    #[test]
    fn rejects_unknown_modes() {
        assert!(matches!(
            SecurityMode::parse("unsafe"),
            Err(SandboxError::InvalidRequest(_))
        ));
    }

    #[test]
    fn only_trusted_mode_allows_current_in_process_runtimes() {
        assert!(SecurityMode::Trusted
            .ensure_in_process_execution_allowed()
            .is_ok());
        assert!(SecurityMode::Restricted
            .ensure_in_process_execution_allowed()
            .is_err());
        assert!(SecurityMode::HostileCode
            .ensure_in_process_execution_allowed()
            .is_err());
    }
}
