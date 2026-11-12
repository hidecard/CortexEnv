//! Linux seccomp-BPF policy compilation and child-thread installation.
//!
//! These policies reduce syscall attack surface but are not a complete sandbox.
//! They must be installed in a dedicated, single-threaded child before loading a
//! runtime. Namespaces, filesystem policy, cgroups, and parent supervision remain
//! separate required controls.

use sandbox_common::SandboxError;
use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, TargetArch};
use std::{collections::BTreeMap, convert::TryInto};

/// Runtime-specific seccomp profile selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompProfile {
    /// Baseline profile for a WASM child supervisor.
    Wasm,
}

/// Compile and install the selected profile on the calling thread.
///
/// The caller must first set `PR_SET_NO_NEW_PRIVS` and must be a dedicated,
/// single-threaded child that has not loaded user code. `seccompiler::apply_filter`
/// applies this filter to the current thread; future threads inherit it.
pub(super) fn apply_seccomp_profile(profile: SeccompProfile) -> Result<(), SandboxError> {
    install_denylist(blocked_syscalls(profile))
}

#[cfg(test)]
fn compile_seccomp_profile(profile: SeccompProfile) -> Result<BpfProgram, SandboxError> {
    compile_denylist(blocked_syscalls(profile))
}

fn blocked_syscalls(profile: SeccompProfile) -> &'static [i64] {
    match profile {
        SeccompProfile::Wasm => &[
            libc::SYS_ptrace,
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_setns,
            libc::SYS_unshare,
            libc::SYS_bpf,
            libc::SYS_perf_event_open,
            libc::SYS_kexec_load,
            libc::SYS_init_module,
            libc::SYS_finit_module,
            libc::SYS_delete_module,
            libc::SYS_reboot,
            libc::SYS_swapon,
            libc::SYS_swapoff,
            libc::SYS_open_by_handle_at,
            libc::SYS_userfaultfd,
            libc::SYS_io_uring_setup,
            libc::SYS_io_uring_enter,
            libc::SYS_io_uring_register,
        ],
    }
}

fn compile_denylist(blocked_syscalls: &[i64]) -> Result<BpfProgram, SandboxError> {
    let rules = blocked_syscalls
        .iter()
        .copied()
        .map(|syscall| (syscall, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let target_arch: TargetArch = std::env::consts::ARCH
        .try_into()
        .map_err(|_| SandboxError::Isolation)?;
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        target_arch,
    )
    .map_err(|_| SandboxError::Isolation)?;

    filter.try_into().map_err(|_| SandboxError::Isolation)
}

fn install_denylist(blocked_syscalls: &[i64]) -> Result<(), SandboxError> {
    let program = compile_denylist(blocked_syscalls)?;
    seccompiler::apply_filter(&program).map_err(|_| SandboxError::Isolation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::unistd::getpid;

    #[test]
    fn wasm_profile_compiles_for_the_host_architecture() {
        assert!(compile_seccomp_profile(SeccompProfile::Wasm).is_ok());
    }

    #[test]
    fn installed_filter_denies_getpid_independent_of_host_privileges() {
        let result = std::thread::spawn(|| {
            capctl::prctl::set_no_new_privs().map_err(|_| SandboxError::Isolation)?;
            install_denylist(&[libc::SYS_getpid])?;
            (getpid().as_raw() == -1)
                .then_some(())
                .ok_or(SandboxError::Isolation)
        })
        .join()
        .expect("seccomp test thread must not panic");

        assert!(result.is_ok(), "the BPF filter must deny getpid");
    }
}
