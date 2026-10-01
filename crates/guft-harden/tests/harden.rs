#![cfg(target_os = "linux")]
//! Restrictions are permanent for a process, so each check runs in a child
//! process (this same test binary re-invoked with an env var).

use std::path::PathBuf;
use std::process::Command;

use guft_harden::{harden, Policy, Report};

const CHILD: &str = "GUFT_HARDEN_CHILD_DIRS";

fn child_main() {
    let dirs: Vec<PathBuf> = std::env::var(CHILD).unwrap().split(':').map(PathBuf::from).collect();
    let (allowed, other) = (dirs[0].clone(), dirs[1].clone());
    let report = harden(&Policy { write_dirs: vec![allowed.clone()], skip_filesystem: false }).expect("harden");
    println!("report: {report:?}");
    let mut failures = Vec::new();

    if !report.dumpable_off || !report.no_new_privs || !report.core_limit_zero || !report.seccomp {
        failures.push(format!("basic protections missing: {report:?}"));
    }
    // SAFETY: integer-only prctl query.
    if unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) } != 0 {
        failures.push("still dumpable".into());
    }

    // Filesystem: write where allowed, refused elsewhere, reading still works.
    if std::fs::write(allowed.join("ok.txt"), b"x").is_err() {
        failures.push("cannot write to the allowed dir".into());
    }
    match std::fs::write(other.join("nope.txt"), b"x") {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {}
        other => failures.push(format!("write outside the allowed dir was not refused: {other:?}")),
    }
    if std::fs::read_to_string("/proc/self/status").is_err() || std::fs::read("/etc/hostname").is_err() && std::fs::read("/etc/passwd").is_err() {
        failures.push("reading the filesystem broke".into());
    }
    // Deleting and creating directories outside is refused too.
    if std::fs::create_dir(other.join("sub")).is_ok() {
        failures.push("could create a directory outside".into());
    }

    // Seccomp: ptrace and cross-process memory reads fail with EPERM.
    // SAFETY: ptrace(PTRACE_TRACEME) takes no pointers; the syscall result is only inspected.
    let r = unsafe { libc::syscall(libc::SYS_ptrace, libc::PTRACE_TRACEME, 0, 0, 0) };
    if r != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
        failures.push("ptrace was not denied".into());
    }
    // SAFETY: with null/zero arguments the kernel rejects before touching memory; we only read errno.
    let r = unsafe { libc::syscall(libc::SYS_process_vm_readv, 1, 0, 0, 0, 0, 0) };
    if r != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
        failures.push("process_vm_readv was not denied".into());
    }

    // Children inherit it, and ordinary programs still run.
    match Command::new("/bin/true").status() {
        Ok(s) if s.success() => {}
        other => failures.push(format!("could not spawn a child: {other:?}")),
    }

    if failures.is_empty() {
        std::process::exit(0);
    }
    eprintln!("FAILURES: {failures:#?}");
    std::process::exit(1);
}

#[test]
fn hardening_takes_effect_in_a_child_process() {
    if std::env::var(CHILD).is_ok() {
        child_main();
    }
    let (allowed, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "hardening_takes_effect_in_a_child_process", "--nocapture", "--test-threads=1"])
        .env(CHILD, format!("{}:{}", allowed.path().display(), other.path().display()))
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "child failed:\n{text}");
    assert!(allowed.path().join("ok.txt").exists());
    assert!(!other.path().join("nope.txt").exists());
    println!("{text}");
}

#[test]
fn report_default_is_all_off() {
    assert_eq!(Report::default(), Report { dumpable_off: false, core_limit_zero: false, no_new_privs: false, filesystem: None, seccomp: false });
}
