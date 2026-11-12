//! Linux isolation controls for a future child-process supervisor.
//!
//! This module deliberately does not mutate the current server process. A caller
//! must create a child supervisor, attach its PID to a cgroup, and apply the
//! namespace/seccomp plan before executing untrusted code.
#![cfg(target_os = "linux")]

mod cgroup;
mod seccomp;
mod supervisor;
pub use cgroup::{detect_cgroup_v2, CgroupLimits, CgroupV2};
pub use seccomp::SeccompProfile;
pub use supervisor::{apply_child_isolation, ChildIsolationConfig};

use crate::ExecutionWorkspace;
use sandbox_common::{ExecutionRequest, ResourceLimits, SandboxError};
use std::path::{Path, PathBuf};
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

/// Configuration used before a child process is launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSupervisorConfig {
    /// cgroup v2 root. When absent, `/sys/fs/cgroup` is detected.
    pub cgroup_root: Option<PathBuf>,
    /// Require a writable per-execution cgroup before launch.
    pub require_cgroup: bool,
}

impl Default for ChildSupervisorConfig {
    fn default() -> Self {
        Self {
            cgroup_root: None,
            require_cgroup: true,
        }
    }
}

/// Resources admitted for one future child execution.
///
/// `prepare` performs all checks that must happen before source is released to
/// a child. It does not launch a process and does not claim that namespaces or
/// seccomp are active. Dropping this value removes the private workspace and
/// tears down the cgroup.
pub struct PreparedExecution {
    request_id: Uuid,
    workspace: ExecutionWorkspace,
    cgroup: Option<CgroupV2>,
    child_pid: Option<u32>,
}

impl std::fmt::Debug for PreparedExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedExecution")
            .field("request_id", &self.request_id)
            .field("workspace", &self.workspace.root())
            .field("cgroup", &self.cgroup.as_ref().map(CgroupV2::path))
            .field("child_pid", &self.child_pid)
            .finish()
    }
}

impl PreparedExecution {
    /// Stable request identifier being admitted.
    pub fn request_id(&self) -> Uuid {
        self.request_id
    }

    /// Private workspace reserved for the future child.
    pub fn workspace(&self) -> &ExecutionWorkspace {
        &self.workspace
    }

    /// cgroup path when a cgroup was successfully prepared.
    pub fn cgroup_path(&self) -> Option<&Path> {
        self.cgroup.as_ref().map(CgroupV2::path)
    }

    /// Attach the supervisor child to the prepared cgroup before releasing source.
    ///
    /// A prepared execution without a cgroup cannot be used as a restricted
    /// child-supervisor admission, so it fails closed instead of silently
    /// running outside resource controls.
    pub fn attach_child(&mut self, pid: u32) -> Result<(), SandboxError> {
        if pid == 0 {
            return Err(SandboxError::Isolation);
        }
        self.cgroup
            .as_ref()
            .ok_or(SandboxError::Isolation)?
            .attach_pid(pid)?;
        self.child_pid = Some(pid);
        Ok(())
    }

    /// Return a permit that authorizes releasing source to the attached child.
    ///
    /// The future supervisor must obtain this permit only after cgroup
    /// attachment and before writing source or starting the runtime.
    pub fn release_source(&self) -> Result<ChildLaunchPermit, SandboxError> {
        let child_pid = self.child_pid.ok_or(SandboxError::Isolation)?;
        Ok(ChildLaunchPermit {
            request_id: self.request_id,
            child_pid,
            workspace: self.workspace.root().to_path_buf(),
        })
    }

    /// Kill the complete execution cgroup during cancellation or cleanup.
    pub fn kill_children(&self) -> Result<(), SandboxError> {
        self.cgroup
            .as_ref()
            .ok_or(SandboxError::Isolation)?
            .kill_all()
    }
}

/// Pre-spawn child admission coordinator.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChildSupervisor;

impl ChildSupervisor {
    /// Validate a request, create its workspace, then create its cgroup.
    ///
    /// The ordering is intentional: invalid requests never mutate host state;
    /// a cgroup is created before a caller can release source to a child; and
    /// missing cgroup support fails closed when required by configuration.
    pub fn prepare(
        &self,
        request: &ExecutionRequest,
        config: &ChildSupervisorConfig,
    ) -> Result<PreparedExecution, SandboxError> {
        request.validate()?;
        let workspace = ExecutionWorkspace::new()?;
        let cgroup_root = config.cgroup_root.clone().or_else(detect_cgroup_v2);
        let cgroup = match (config.require_cgroup, cgroup_root) {
            (true, Some(root)) => Some(CgroupV2::create(root, request.id, &request.limits)?),
            (true, None) => return Err(SandboxError::Isolation),
            (false, Some(root)) => CgroupV2::create(root, request.id, &request.limits).ok(),
            (false, None) => None,
        };
        Ok(PreparedExecution {
            request_id: request.id,
            workspace,
            cgroup,
            child_pid: None,
        })
    }
}

/// Capability proving that a prepared child passed the attach gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildLaunchPermit {
    request_id: Uuid,
    child_pid: u32,
    workspace: PathBuf,
}

impl ChildLaunchPermit {
    /// Request associated with the permit.
    pub fn request_id(&self) -> Uuid {
        self.request_id
    }

    /// Attached child PID authorized to receive source.
    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    /// Workspace reserved for the authorized child.
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
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

    #[test]
    fn admission_rejects_missing_cgroup_before_source_release() {
        let request = ExecutionRequest {
            id: Uuid::new_v4(),
            language: sandbox_common::Language::Wasm,
            code: "0061736d01000000".into(),
            limits: ResourceLimits::default(),
            network: sandbox_common::NetworkPolicy::Deny,
        };
        let config = ChildSupervisorConfig {
            cgroup_root: Some(PathBuf::from("/definitely/missing/cortexenv-cgroup")),
            require_cgroup: true,
        };
        assert!(matches!(
            ChildSupervisor.prepare(&request, &config),
            Err(SandboxError::Isolation)
        ));
    }

    #[test]
    fn prepared_execution_attaches_and_kills_the_child_cgroup() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("cgroup.controllers"), "cpu memory pids").unwrap();
        let request = ExecutionRequest {
            id: Uuid::new_v4(),
            language: sandbox_common::Language::Wasm,
            code: "0061736d01000000".into(),
            limits: ResourceLimits::default(),
            network: sandbox_common::NetworkPolicy::Deny,
        };
        let mut prepared = ChildSupervisor
            .prepare(
                &request,
                &ChildSupervisorConfig {
                    cgroup_root: Some(root.path().to_path_buf()),
                    require_cgroup: true,
                },
            )
            .unwrap();
        let cgroup = prepared.cgroup_path().unwrap().to_path_buf();
        std::fs::write(cgroup.join("cgroup.procs"), "").unwrap();
        std::fs::write(cgroup.join("cgroup.kill"), "").unwrap();
        prepared.attach_child(4242).unwrap();
        assert_eq!(
            std::fs::read_to_string(cgroup.join("cgroup.procs")).unwrap(),
            "4242"
        );
        prepared.kill_children().unwrap();
        assert_eq!(
            std::fs::read_to_string(cgroup.join("cgroup.kill")).unwrap(),
            "1"
        );
    }

    #[test]
    fn source_release_requires_a_valid_attached_child() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("cgroup.controllers"), "cpu memory pids").unwrap();
        let request = ExecutionRequest {
            id: Uuid::new_v4(),
            language: sandbox_common::Language::Wasm,
            code: "0061736d01000000".into(),
            limits: ResourceLimits::default(),
            network: sandbox_common::NetworkPolicy::Deny,
        };
        let mut prepared = ChildSupervisor
            .prepare(
                &request,
                &ChildSupervisorConfig {
                    cgroup_root: Some(root.path().to_path_buf()),
                    require_cgroup: true,
                },
            )
            .unwrap();
        assert!(matches!(
            prepared.release_source(),
            Err(SandboxError::Isolation)
        ));
        let cgroup = prepared.cgroup_path().unwrap().to_path_buf();
        std::fs::write(cgroup.join("cgroup.procs"), "").unwrap();
        std::fs::write(cgroup.join("cgroup.kill"), "").unwrap();
        assert!(prepared.attach_child(0).is_err());
        prepared.attach_child(4243).unwrap();
        let permit = prepared.release_source().unwrap();
        assert_eq!(permit.request_id(), request.id);
        assert_eq!(permit.child_pid(), 4243);
        assert_eq!(permit.workspace(), prepared.workspace().root());
    }
}
