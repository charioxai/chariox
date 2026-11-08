//! Stack-size checks that match the kernel binary, which runs without `RUST_MIN_STACK`.

const CHILD: &str = "CHARIOX_STACK_CHECK_CHILD";

/// The stack size of the thread the caller runs on.
pub(crate) fn current_thread_stack_size() -> usize {
    // SAFETY: the calls only read the calling thread's own attributes, and the attribute
    // object is destroyed before returning.
    unsafe {
        let mut attributes: libc::pthread_attr_t = std::mem::zeroed();
        assert_eq!(
            libc::pthread_getattr_np(libc::pthread_self(), &mut attributes),
            0
        );
        let mut address = std::ptr::null_mut();
        let mut size = 0;
        assert_eq!(
            libc::pthread_attr_getstack(&attributes, &mut address, &mut size),
            0
        );
        libc::pthread_attr_destroy(&mut attributes);
        size
    }
}

/// Runs the named test again in a child test process without `RUST_MIN_STACK` and returns
/// `false` once the child passed. Returns `true` in the child, where the caller runs its
/// checks. The test suite sets 16 MiB, which hides stack overflows the kernel would hit.
pub(crate) fn is_stack_check_child(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture"])
        .env(CHILD, "1")
        .env_remove("RUST_MIN_STACK")
        .status()
        .unwrap();
    assert!(status.success(), "{test} failed without RUST_MIN_STACK");
    false
}
