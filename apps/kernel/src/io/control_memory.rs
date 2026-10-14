//! MP-08 / MP-10 / MP-11: kernel-owned readers protect control and scanned memory.
#[cfg(target_os = "linux")]
pub(crate) fn make_process_nondumpable() -> bool {
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) == 0 }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn make_process_nondumpable() -> bool {
    true
}
