//! Linux isolation controls for a future child-process supervisor.
//!
//! This module deliberately does not mutate the current server process. A caller
//! must create a child supervisor, attach its PID to a cgroup, and apply the
//! namespace/seccomp plan before executing untrusted code.
#![cfg(target_os = "linux")]

use sandbox_common::{ResourceLimits, SandboxError};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

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

/// Resource values written to cgroup v2 controller files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupLimits {
    /// `memory.max` in bytes.
    pub memory_max: u64,
    /// `memory.swap.max` in bytes. Zero disables swap for the sandbox.
    pub swap_max: u64,
    /// `pids.max` process/thread count.
    pub pids_max: u32,
}

/// Runtime-specific seccomp profile selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompProfile {
    /// Minimal profile for a WASM child supervisor.
    Wasm,
}

/// A cgroup v2 directory owned by one execution.
#[derive(Debug)]
pub struct CgroupV2 {
    path: PathBuf,
}

impl CgroupV2 {
    /// Create an execution cgroup below a cgroup v2 mount and write hard limits.
    pub fn create(
        root: impl AsRef<Path>,
        id: Uuid,
        limits: &ResourceLimits,
    ) -> Result<Self, SandboxError> {
        limits.validate()?;
        let root = root.as_ref();
        if !root.join("cgroup.controllers").is_file() {
            return Err(SandboxError::Isolation);
        }
        let path = root.join(format!("cortexenv-{id}"));
        fs::create_dir(&path).map_err(|_| SandboxError::Isolation)?;
        let result = (|| {
            write_value(&path.join("memory.max"), &limits.memory_bytes.to_string())?;
            write_value(&path.join("memory.swap.max"), "0")?;
            write_value(&path.join("pids.max"), &limits.process_count.to_string())?;
            Ok(Self { path: path.clone() })
        })();
        if result.is_err() {
            let _ = fs::remove_dir(&path);
        }
        result
    }

    /// Attach a child PID to this execution cgroup.
    pub fn attach_pid(&self, pid: u32) -> Result<(), SandboxError> {
        write_value(&self.path.join("cgroup.procs"), &pid.to_string())
    }

    /// Return the cgroup directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CgroupV2 {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

/// Return the conventional unified cgroup v2 mount when available.
pub fn detect_cgroup_v2() -> Option<PathBuf> {
    let root = PathBuf::from("/sys/fs/cgroup");
    root.join("cgroup.controllers").is_file().then_some(root)
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
        cgroup: CgroupLimits {
            memory_max: limits.memory_bytes,
            swap_max: 0,
            pids_max: limits.process_count,
        },
        seccomp_profile: SeccompProfile::Wasm,
    })
}

fn write_value(path: &Path, value: &str) -> Result<(), SandboxError> {
    fs::write(path, value).map_err(|_| SandboxError::Isolation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn plan_is_network_disabled_and_has_hard_limits() {
        let limits = ResourceLimits::default();
        let plan = isolation_plan(&limits).unwrap();
        assert!(plan.network_disabled);
        assert_eq!(plan.cgroup.memory_max, limits.memory_bytes);
        assert_eq!(plan.cgroup.swap_max, 0);
        assert_eq!(plan.cgroup.pids_max, limits.process_count);
    }

    #[test]
    fn cgroup_creation_fails_closed_without_v2_controller() {
        let root = tempdir().unwrap();
        assert!(CgroupV2::create(root.path(), Uuid::new_v4(), &ResourceLimits::default()).is_err());
    }
}
