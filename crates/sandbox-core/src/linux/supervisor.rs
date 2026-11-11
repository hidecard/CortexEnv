//! Fail-closed child-side Linux hardening primitives.
//!
//! These functions are intentionally child-only. They must be called after a
//! dedicated child has been created and before the child reads or executes user
//! source. They never run from an HTTP handler or mutate the API process.

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
    /// Drop effective, permitted, inheritable, ambient, and bounding capabilities.
    pub drop_capabilities: bool,
}

impl Default for ChildIsolationConfig {
    fn default() -> Self {
        Self {
            namespaces: true,
            require_seccomp: true,
            drop_capabilities: true,
        }
    }
}

/// Apply child-side namespace, rlimit, no-new-privileges, and capability controls.
///
/// The function is deliberately fail-closed: the repository's pinned
/// `seccompiler` dependency is currently a placeholder crate and cannot install
/// a real BPF filter. When `require_seccomp` is true, this function returns
/// [`SandboxError::Isolation`] without launching a runtime. This makes the
/// missing filter an explicit deployment failure rather than a false claim that
/// seccomp is active.
///
/// A future supervisor must call this from a dedicated child before it starts a
/// language runtime, then replace the backend check with a real per-runtime
/// filter and only continue after that filter has been installed successfully.
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
        // Do not use PR_SET_SECCOMP strict mode here: it would kill normal
        // runtimes because it permits only read/write/exit/sigreturn. The
        // pinned seccompiler crate has no filter API, so fail closed instead.
        return Err(SandboxError::Isolation);
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
