//! Linux isolation controls for a future child-process supervisor.
//!
//! This module deliberately does not mutate the current server process. A caller
//! must create a child supervisor, attach its PID to a cgroup, and apply the
//! namespace/seccomp plan before executing untrusted code.
#![cfg(target_os = "linux")]

mod cgroup;
pub use cgroup::{detect_cgroup_v2, CgroupLimits, CgroupV2};

use sandbox_common::{ResourceLimits, SandboxError};

/// Linux isolation settings that a child supervisor must apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxIsolationPlan {
    /// Namespace flags requested for the child process.
    pub namespace_flags: u64,
    /// Network access remains disabled by default.
    pub network_disabled: bool,
    /// cgroup v2 limits.
    pub cgroup: CgroupLimits,
    /// Seccomp profile name selected for the runtime.
    pub seccomp_profile: SeccompProfile,
}

/// Runtime-specific seccomp profile selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompProfile {
    /// Minimal profile for a WASM child supervisor.
    Wasm,
}

/// Build the fail-closed plan without changing process state.
pub fn isolation_plan(limits: &ResourceLimits) -> Result<LinuxIsolationPlan, SandboxError> {
    limits.validate()?;
    Ok(LinuxIsolationPlan {
        // CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWPID | CLONE_NEWNET |
        // CLONE_NEWUTS | CLONE_NEWIPC. The supervisor converts these to
        // platform constants before calling unshare/clone.
        namespace_flags: 0x10000000
            | 0x00020000
            | 0x20000000
            | 0x40000000
            | 0x04000000
            | 0x08000000,
        network_disabled: true,
        cgroup: CgroupLimits::from_resource_limits(limits),
        seccomp_profile: SeccompProfile::Wasm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_is_network_disabled_and_has_hard_limits() {
        let limits = ResourceLimits::default();
        let plan = isolation_plan(&limits).unwrap();
        assert!(plan.network_disabled);
        assert_eq!(plan.cgroup.memory_max, limits.memory_bytes);
        assert_eq!(plan.cgroup.swap_max, 0);
        assert_eq!(plan.cgroup.pids_max, limits.process_count);
        assert_eq!(plan.cgroup.cpu_quota_micros, 100_000);
        assert_eq!(plan.cgroup.cpu_period_micros, 100_000);
    }
}
