//! MP-08 / MP-09 / MP-10 / MP-11: the stack a kernel thread really runs on.
use std::os::unix::thread::JoinHandleExt;

const CHILD: &str = "CHARIOX_THREAD_STACK_CHILD";

/// The stack size of the running thread behind `handle`.
pub(crate) fn spawned_thread_stack_size<T>(handle: &std::thread::JoinHandle<T>) -> usize {
    stack_size(handle.as_pthread_t())
}

/// The stack size of the thread the caller runs on.
pub(crate) fn current_thread_stack_size() -> usize {
    // SAFETY: `pthread_self` has no preconditions.
    stack_size(unsafe { libc::pthread_self() })
}

fn stack_size(thread: libc::pthread_t) -> usize {
    // SAFETY: the callers pass a live thread, the calls only read its attributes, and the
    // attribute object is destroyed before returning.
    unsafe {
        let mut attributes: libc::pthread_attr_t = std::mem::zeroed();
        assert_eq!(libc::pthread_getattr_np(thread, &mut attributes), 0);
        let mut size = 0;
        assert_eq!(libc::pthread_attr_getstacksize(&attributes, &mut size), 0);
        libc::pthread_attr_destroy(&mut attributes);
        size
    }
}

/// The kernel binary runs without `RUST_MIN_STACK`, which the test runner sets. Reruns `test`
/// in a child test process without it and returns false; returns true inside that child, where
/// the caller checks the stack.
pub(crate) fn rerun_without_min_stack(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads", "1"])
        .env(CHILD, "1")
        .env_remove("RUST_MIN_STACK")
        .status()
        .unwrap();
    assert!(status.success(), "{test} failed without RUST_MIN_STACK");
    false
}
