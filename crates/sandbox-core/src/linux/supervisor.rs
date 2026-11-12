//! Fail-closed child-side Linux hardening primitives.
//!
//! These functions are intentionally child-only. They must be called after a
//! dedicated child has been created and before the child reads or executes user
//! source. They never run from an HTTP handler or mutate the API process.

use super::{seccomp::apply_seccomp_profile, SeccompProfile};
use capctl::{caps::CapState, prctl};
use nix::{
    sched::{unshare, CloneFlags},
    sys::resource::{rlim_t, setrlimit, Resource},
};
use sandbox_common::{ResourceLimits, SandboxError};

/// Child-side hardening options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildIsolationConfig {
    /// Create user, mount, PID, network, UTS, and IPC namespaces.
    pub namespaces: bool,
    /// Require a usable seccomp backend before allowing execution.
    pub require_seccomp: bool,
    /// Runtime-specific seccomp profile to install in the child.
    pub seccomp_profile: SeccompProfile,
    /// Drop effective, permitted, inheritable, ambient, and bounding capabilities.
    pub drop_capabilities: bool,
}

impl Default for ChildIsolationConfig {
    fn default() -> Self {
        Self {
            namespaces: true,
            require_seccomp: true,
            seccomp_profile: SeccompProfile::Wasm,
            drop_capabilities: true,
        }
    }
}

/// Apply child-side namespace, rlimit, no-new-privileges, capability, and seccomp controls.
///
/// This function must run in a dedicated, single-threaded child before the
/// runtime or user source is loaded. It installs the selected BPF filter only
/// on the calling thread; new child threads inherit it. Any missing control or
/// installation failure is returned as [`SandboxError::Isolation`].
#[cfg(target_os = "linux")]
pub fn apply_child_isolation(
    limits: &ResourceLimits,
    config: ChildIsolationConfig,
) -> Result<(), SandboxError> {
    limits.validate()?;

    if config.namespaces {
        unshare(
            CloneFlags::CLONE_NEWUSER
                | CloneFlags::CLONE_NEWNS
                | CloneFlags::CLONE_NEWPID
                | CloneFlags::CLONE_NEWNET
                | CloneFlags::CLONE_NEWUTS
                | CloneFlags::CLONE_NEWIPC,
        )
        .map_err(|_| SandboxError::Isolation)?;
    }

    setrlimit(
        Resource::RLIMIT_NOFILE,
        limits.file_descriptors as rlim_t,
        limits.file_descriptors as rlim_t,
    )
    .map_err(|_| SandboxError::Isolation)?;

    prctl::set_no_new_privs().map_err(|_| SandboxError::Isolation)?;

    if config.drop_capabilities {
        CapState::empty()
            .set_current()
            .map_err(|_| SandboxError::Isolation)?;
        capctl::caps::ambient::clear().map_err(|_| SandboxError::Isolation)?;
        capctl::caps::bounding::clear().map_err(|_| SandboxError::Isolation)?;
    }

    if config.require_seccomp {
        apply_seccomp_profile(config.seccomp_profile)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_require_all_child_controls() {
        let config = ChildIsolationConfig::default();
        assert!(config.namespaces);
        assert!(config.require_seccomp);
        assert_eq!(config.seccomp_profile, SeccompProfile::Wasm);
        assert!(config.drop_capabilities);
    }

    #[test]
    fn invalid_limits_fail_before_namespace_mutation() {
        let mut limits = ResourceLimits::default();
        limits.file_descriptors = 0;
        let error = apply_child_isolation(&limits, ChildIsolationConfig::default())
            .expect_err("invalid limits must fail closed");
        assert!(matches!(error, SandboxError::InvalidRequest(_)));
    }
}
