//! Process hardening: a second sandbox layer under Rust's memory safety.
//!
//! Applied once at startup, before any secret is loaded:
//!
//! * no core dumps, and not attachable by other processes (`PR_SET_DUMPABLE=0`);
//! * `no_new_privs`, so nothing we spawn can gain privileges;
//! * Landlock: the filesystem is read-only except for the directories we name;
//! * seccomp: syscalls with no legitimate use here (ptrace, reading other
//!   processes' memory, kernel modules, bpf, ...) fail with `EPERM`.
//!
//! The seccomp filter is inherited by child processes, so it deliberately leaves
//! alone everything the WebView's own sandbox needs (namespaces, mount, clone).
//! Everything is best-effort across kernel versions, and the returned
//! [`Report`] says what actually took effect. Restrictions cannot be undone.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Os(String),
}

/// What to allow. Everything else on disk becomes read-only.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// Directories (and their contents) we may write to.
    pub write_dirs: Vec<PathBuf>,
    /// Skip Landlock (for environments where it breaks the toolkit).
    pub skip_filesystem: bool,
}

/// What took effect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub dumpable_off: bool,
    pub core_limit_zero: bool,
    pub no_new_privs: bool,
    pub filesystem: Option<&'static str>,
    pub seccomp: bool,
}

fn os_err(what: &str) -> Error {
    Error::Os(format!("{what}: {}", std::io::Error::last_os_error()))
}

#[cfg(target_os = "linux")]
pub fn harden(policy: &Policy) -> Result<Report, Error> {
    let mut report = Report::default();

    // SAFETY: plain syscalls with integer arguments; no pointers are involved.
    unsafe {
        report.dumpable_off = libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) == 0;
        report.no_new_privs = libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) == 0;
    }
    let zero = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: `zero` is a valid, initialized rlimit that outlives the call.
    report.core_limit_zero = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &zero) } == 0;
    if !report.dumpable_off || !report.no_new_privs {
        return Err(os_err("prctl"));
    }

    if !policy.skip_filesystem {
        report.filesystem = Some(landlock_restrict(&policy.write_dirs)?);
    }
    seccomp_deny()?;
    report.seccomp = true;
    Ok(report)
}

#[cfg(not(target_os = "linux"))]
pub fn harden(_: &Policy) -> Result<Report, Error> {
    Ok(Report::default())
}

#[cfg(target_os = "linux")]
fn landlock_restrict(write_dirs: &[PathBuf]) -> Result<&'static str, Error> {
    use landlock::{
        path_beneath_rules, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, ABI,
    };
    let abi = ABI::V5;
    let read = AccessFs::from_read(abi);
    let all = AccessFs::from_all(abi);
    let mut existing = Vec::new();
    for d in write_dirs {
        std::fs::create_dir_all(d).map_err(|e| Error::Os(format!("create {}: {e}", d.display())))?;
        existing.push(d.clone());
    }
    let status = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(all)
        .map_err(|e| Error::Os(e.to_string()))?
        .create()
        .map_err(|e| Error::Os(e.to_string()))?
        // Read (and execute) anywhere; write only where allowed.
        .add_rules(path_beneath_rules(["/"], read))
        .map_err(|e| Error::Os(e.to_string()))?
        .add_rules(path_beneath_rules(existing, all))
        .map_err(|e| Error::Os(e.to_string()))?
        .restrict_self()
        .map_err(|e| Error::Os(e.to_string()))?;
    Ok(match status.ruleset {
        RulesetStatus::FullyEnforced => "fully enforced",
        RulesetStatus::PartiallyEnforced => "partially enforced",
        RulesetStatus::NotEnforced => "not enforced (kernel lacks Landlock)",
    })
}

#[cfg(target_os = "linux")]
fn seccomp_deny() -> Result<(), Error> {
    use seccompiler::{apply_filter, BpfProgram, SeccompAction, SeccompFilter, TargetArch};
    use std::collections::BTreeMap;

    // Nothing here is needed by this app, Tor, SQLite or WebKitGTK.
    let denied: &[i64] = &[
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_kcmp,
        libc::SYS_kexec_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_acct,
        libc::SYS_settimeofday,
        libc::SYS_clock_settime,
        libc::SYS_adjtimex,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_open_by_handle_at,
        libc::SYS_name_to_handle_at,
    ];
    let rules: BTreeMap<i64, Vec<seccompiler::SeccompRule>> = denied.iter().map(|s| (*s, vec![])).collect();
    let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(|e| Error::Os(format!("seccomp arch: {e}")))?;
    let filter = SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(libc::EPERM as u32), arch)
        .map_err(|e| Error::Os(format!("seccomp filter: {e}")))?;
    let program: BpfProgram = filter.try_into().map_err(|e| Error::Os(format!("seccomp compile: {e}")))?;
    apply_filter(&program).map_err(|e| Error::Os(format!("seccomp apply: {e}")))
}
