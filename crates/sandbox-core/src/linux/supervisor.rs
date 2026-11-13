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
    unistd::{getegid, geteuid},
};
use sandbox_common::{ResourceLimits, SandboxError};
use std::{fs, path::Path};

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
    /// Map the launching uid/gid to an unprivileged identity in the user namespace.
    pub map_user_namespace: bool,
}

impl Default for ChildIsolationConfig {
    fn default() -> Self {
        Self {
            namespaces: true,
            require_seccomp: true,
            seccomp_profile: SeccompProfile::Wasm,
            drop_capabilities: true,
            map_user_namespace: true,
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
        unshare(CloneFlags::CLONE_NEWUSER).map_err(|_| SandboxError::Isolation)?;
        if config.map_user_namespace {
            map_current_user_namespace()?;
        }
        unshare(
            CloneFlags::CLONE_NEWNS
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

/// Map the current process identity to uid/gid 0 inside its fresh user namespace.
///
/// The mapping is deliberately one-to-one with the launching host identity: uid 0
/// is only namespace-local and the capability bounding set is cleared later in the
/// same child. `setgroups` is denied before writing `gid_map`, as required by the
/// kernel for an unprivileged gid mapping.
#[cfg(target_os = "linux")]
fn map_current_user_namespace() -> Result<(), SandboxError> {
    let uid = geteuid().as_raw();
    let gid = getegid().as_raw();
    let setgroups = Path::new("/proc/self/setgroups");
    if setgroups.exists() {
        fs::write(setgroups, "deny").map_err(|_| SandboxError::Isolation)?;
    }
    write_mapping("/proc/self/uid_map", uid)?;
    write_mapping("/proc/self/gid_map", gid)
}

#[cfg(target_os = "linux")]
fn write_mapping(path: &str, host_id: u32) -> Result<(), SandboxError> {
    fs::write(path, format!("0 {host_id} 1\n")).map_err(|_| SandboxError::Isolation)
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
        assert!(config.map_user_namespace);
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
