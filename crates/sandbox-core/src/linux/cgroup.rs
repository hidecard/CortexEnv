//! cgroup v2 resource controller setup.

use sandbox_common::{ResourceLimits, SandboxError};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const CPU_PERIOD_MICROS: u64 = 100_000;

/// Resource values written to cgroup v2 controller files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupLimits {
    /// `memory.max` in bytes.
    pub memory_max: u64,
    /// `memory.swap.max` in bytes. Zero disables swap for the sandbox.
    pub swap_max: u64,
    /// `pids.max` process/thread count.
    pub pids_max: u32,
    /// `cpu.max` quota in microseconds per period.
    pub cpu_quota_micros: u64,
    /// `cpu.max` period in microseconds.
    pub cpu_period_micros: u64,
}

impl CgroupLimits {
    pub(crate) fn from_resource_limits(limits: &ResourceLimits) -> Self {
        Self {
            memory_max: limits.memory_bytes,
            swap_max: 0,
            pids_max: limits.process_count,
            cpu_quota_micros: u64::from(limits.cpu_quota_millis) * CPU_PERIOD_MICROS / 1_000,
            cpu_period_micros: CPU_PERIOD_MICROS,
        }
    }

    fn cpu_max_value(&self) -> String {
        format!("{} {}", self.cpu_quota_micros, self.cpu_period_micros)
    }
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
        let result = apply_limits(&path, &CgroupLimits::from_resource_limits(limits))
            .map(|()| Self { path: path.clone() });
        if result.is_err() {
            let _ = fs::remove_dir(&path);
        }
        result
    }

    /// Attach a child PID to this execution cgroup.
    pub fn attach_pid(&self, pid: u32) -> Result<(), SandboxError> {
        write_value(&self.path.join("cgroup.procs"), &pid.to_string())
    }

    /// Kill all processes in this cgroup when the kernel exposes `cgroup.kill`.
    pub fn kill_all(&self) -> Result<(), SandboxError> {
        write_value(&self.path.join("cgroup.kill"), "1")
    }

    /// Return the cgroup directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CgroupV2 {
    fn drop(&mut self) {
        let _ = self.kill_all();
        for _ in 0..20 {
            let Ok(events) = fs::read_to_string(self.path.join("cgroup.events")) else {
                break;
            };
            if events.lines().any(|line| line.trim() == "populated 0") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = fs::remove_dir(&self.path);
    }
}

/// Return the conventional unified cgroup v2 mount when available.
pub fn detect_cgroup_v2() -> Option<PathBuf> {
    let root = PathBuf::from("/sys/fs/cgroup");
    root.join("cgroup.controllers").is_file().then_some(root)
}

fn apply_limits(path: &Path, limits: &CgroupLimits) -> Result<(), SandboxError> {
    write_value(&path.join("memory.max"), &limits.memory_max.to_string())?;
    write_value(&path.join("memory.swap.max"), &limits.swap_max.to_string())?;
    write_value(&path.join("pids.max"), &limits.pids_max.to_string())?;
    write_value(&path.join("cpu.max"), &limits.cpu_max_value())
}

fn write_value(path: &Path, value: &str) -> Result<(), SandboxError> {
    let mut controller = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|_| SandboxError::Isolation)?;
    controller
        .write_all(value.as_bytes())
        .map_err(|_| SandboxError::Isolation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cpu_quota_is_converted_to_cgroup_period_and_quota() {
        let mut request_limits = ResourceLimits::default();
        request_limits.cpu_quota_millis = 250;
        let cgroup = CgroupLimits::from_resource_limits(&request_limits);

        assert_eq!(cgroup.cpu_quota_micros, 25_000);
        assert_eq!(cgroup.cpu_period_micros, 100_000);
        assert_eq!(cgroup.cpu_max_value(), "25000 100000");
    }

    #[test]
    fn writes_cpu_and_existing_cgroup_limits() {
        let root = tempdir().unwrap();
        for controller in ["memory.max", "memory.swap.max", "pids.max", "cpu.max"] {
            fs::write(root.path().join(controller), "").unwrap();
        }
        let mut request_limits = ResourceLimits::default();
        request_limits.cpu_quota_millis = 1_500;
        let cgroup = CgroupLimits::from_resource_limits(&request_limits);

        apply_limits(root.path(), &cgroup).unwrap();

        assert_eq!(
            fs::read_to_string(root.path().join("memory.max")).unwrap(),
            "134217728"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("memory.swap.max")).unwrap(),
            "0"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("pids.max")).unwrap(),
            "32"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("cpu.max")).unwrap(),
            "150000 100000"
        );
    }

    #[test]
    fn kill_all_uses_the_cgroup_kill_controller() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("cgroup.kill"), "").unwrap();
        let cgroup = CgroupV2 {
            path: root.path().to_path_buf(),
        };

        cgroup.kill_all().unwrap();

        assert_eq!(
            fs::read_to_string(root.path().join("cgroup.kill")).unwrap(),
            "1"
        );
    }

    #[test]
    fn cgroup_creation_fails_closed_without_v2_controller() {
        let root = tempdir().unwrap();
        assert!(CgroupV2::create(root.path(), Uuid::new_v4(), &ResourceLimits::default()).is_err());
    }
}
