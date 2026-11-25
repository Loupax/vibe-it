//! QNET1: scratchy cannot physically speak to the internet. At startup of the
//! real binary (before any thread or library) a seccomp-BPF filter makes
//! `socket()` / `socketpair()` fail with EAFNOSUPPORT for every address family
//! except AF_UNIX. All networking goes through wire's unix socket. The filter is
//! inherited by every thread and every dlopen'd library (libmpv, ffmpeg).
//! Hand-assembled BPF over `libc` (no `seccompiler` in the registry). No escape
//! hatch. If the kernel refuses (EINVAL/ENOSYS/EPERM) a warning is logged and
//! scratchy continues unfiltered.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Filter installed.
    Installed,
    /// Not supported on this OS or architecture.
    Unsupported,
    /// The kernel refused (the errno).
    Failed(i32),
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod imp {
    use super::Status;

    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xC000_003E;
    #[cfg(target_arch = "x86_64")]
    const NR_SOCKET: u32 = 41;
    #[cfg(target_arch = "x86_64")]
    const NR_SOCKETPAIR: u32 = 53;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xC000_00B7;
    #[cfg(target_arch = "aarch64")]
    const NR_SOCKET: u32 = 198;
    #[cfg(target_arch = "aarch64")]
    const NR_SOCKETPAIR: u32 = 199;
    /// io_uring can create sockets without `socket()`: refuse it (same number on both).
    const NR_IO_URING_SETUP: u32 = 425;
    #[cfg(target_arch = "x86_64")]
    const X32_BIT: u32 = 0x4000_0000;

    const BPF_LD_W_ABS: u16 = 0x20;
    const BPF_JEQ_K: u16 = 0x15;
    #[cfg(target_arch = "x86_64")]
    const BPF_JSET_K: u16 = 0x45;
    const BPF_RET_K: u16 = 0x06;
    const RET_ALLOW: u32 = 0x7fff_0000;
    const RET_KILL_PROCESS: u32 = 0x8000_0000;
    const RET_ERRNO: u32 = 0x0005_0000;
    const OFF_NR: u32 = 0;
    const OFF_ARCH: u32 = 4;
    const OFF_ARG0: u32 = 16;

    fn ins(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
    }

    fn program() -> Vec<libc::sock_filter> {
        let deny_af = RET_ERRNO | libc::EAFNOSUPPORT as u32;
        let mut p = vec![
            ins(BPF_LD_W_ABS, 0, 0, OFF_ARCH),
            // wrong ABI (32-bit compat on x86_64, ...): kill, it could dodge the number checks
            ins(BPF_JEQ_K, 1, 0, AUDIT_ARCH),
            ins(BPF_RET_K, 0, 0, RET_KILL_PROCESS),
            ins(BPF_LD_W_ABS, 0, 0, OFF_NR),
        ];
        #[cfg(target_arch = "x86_64")]
        {
            // x32 ABI calls carry bit 30 of the number: kill them
            p.push(ins(BPF_JSET_K, 0, 1, X32_BIT));
            p.push(ins(BPF_RET_K, 0, 0, RET_KILL_PROCESS));
        }
        p.extend([
            ins(BPF_JEQ_K, 3, 0, NR_SOCKET),
            ins(BPF_JEQ_K, 2, 0, NR_SOCKETPAIR),
            ins(BPF_JEQ_K, 5, 0, NR_IO_URING_SETUP),
            ins(BPF_RET_K, 0, 0, RET_ALLOW),
            // socket / socketpair: the domain (arg0) must be AF_UNIX
            ins(BPF_LD_W_ABS, 0, 0, OFF_ARG0),
            ins(BPF_JEQ_K, 1, 0, libc::AF_UNIX as u32),
            ins(BPF_RET_K, 0, 0, deny_af),
            ins(BPF_RET_K, 0, 0, RET_ALLOW),
            // io_uring_setup
            ins(BPF_RET_K, 0, 0, RET_ERRNO | libc::ENOSYS as u32),
        ]);
        p
    }

    pub(super) fn install() -> Status {
        let prog = program();
        let fprog = libc::sock_fprog { len: prog.len() as u16, filter: prog.as_ptr() as *mut libc::sock_filter };
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Status::Failed(*libc::__errno_location());
            }
            if libc::prctl(libc::PR_SET_SECCOMP, 2 /* SECCOMP_MODE_FILTER */, &fprog as *const libc::sock_fprog, 0, 0) != 0 {
                return Status::Failed(*libc::__errno_location());
            }
        }
        Status::Installed
    }
}

#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
mod imp {
    use super::Status;
    pub(super) fn install() -> Status {
        Status::Unsupported
    }
}

/// Install the filter and log the outcome once to stderr. Call first thing in
/// `main`, before any thread is spawned or library loaded.
pub fn install_and_log() -> Status {
    let s = imp::install();
    match s {
        Status::Installed => eprintln!("scratchy: network: unix sockets only (wire)"),
        Status::Unsupported => eprintln!("scratchy: warning: network lock (seccomp) is not supported on this platform; continuing without it"),
        Status::Failed(e) => eprintln!("scratchy: warning: could not install the network lock (seccomp, errno {e}); continuing without it"),
    }
    s
}

/// Install without logging (test child processes).
pub fn install() -> Status {
    imp::install()
}
