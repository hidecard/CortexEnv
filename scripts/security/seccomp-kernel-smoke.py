#!/usr/bin/env python3
"""Safely probe kernel seccomp primitives in a disposable child process.

This does not claim that CortexEnv's runtime-specific BPF profile is installed.
It only checks that the kernel permits PR_SET_NO_NEW_PRIVS followed by the
minimal SECCOMP_MODE_STRICT filter. The strict filter is installed after fork
and the parent remains unrestricted.
"""

import ctypes
import errno
import os
import sys

LIBC = ctypes.CDLL(None, use_errno=True)
PR_SET_NO_NEW_PRIVS = 38
PR_SET_SECCOMP = 22
SECCOMP_MODE_STRICT = 1


def prctl(option: int, arg2: int) -> None:
    result = LIBC.prctl(option, ctypes.c_ulong(arg2), 0, 0, 0)
    if result != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def main() -> int:
    read_fd, write_fd = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(read_fd)
        try:
            prctl(PR_SET_NO_NEW_PRIVS, 1)
            prctl(PR_SET_SECCOMP, SECCOMP_MODE_STRICT)
            os.write(write_fd, b"seccomp-strict-child-ok\n")
            os._exit(0)
        except OSError as error:
            # Send a machine-readable errno to the parent without requiring
            # Python to perform any syscall after strict mode was installed.
            try:
                os.write(write_fd, f"ERRNO={error.errno}\n".encode())
            finally:
                os._exit(77 if error.errno in (errno.EINVAL, errno.ENOSYS, errno.EPERM) else 1)
    os.close(write_fd)
    message = os.read(read_fd, 4096)
    _, status = os.waitpid(pid, 0)
    # Some kernels/container runtimes terminate the child when Python's
    # interpreter attempts its final exit_group syscall. The marker was
    # written only after PR_SET_SECCOMP succeeded, so a signal after that
    # marker is still positive evidence that strict mode was installed.
    if message.startswith(b"seccomp-strict-child-ok") and (
        (os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0)
        or os.WIFSIGNALED(status)
    ):
        print("PASS: child installed PR_SET_NO_NEW_PRIVS and SECCOMP_MODE_STRICT")
        print("NOTE: this is a kernel primitive smoke test, not CortexEnv's runtime profile")
        return 0
    if os.WIFEXITED(status) and os.WEXITSTATUS(status) == 77:
        print(f"SKIP: kernel/container refused the seccomp probe ({message.decode().strip()})")
        return 77
    print(f"FAIL: seccomp child exited unexpectedly (status={status}, output={message!r})")
    return 1


if __name__ == "__main__":
    sys.exit(main())
