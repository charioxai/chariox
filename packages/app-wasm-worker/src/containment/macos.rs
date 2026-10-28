//! Reuses the v1 worker's parameter-free Seatbelt entry and signed JIT policy.
use std::{
    ffi::{c_char, c_int, c_void, CString},
    io,
};

#[link(name = "sandbox")]
unsafe extern "C" {
    fn sandbox_init_with_parameters(
        profile: *const c_char,
        flags: u64,
        parameters: *const *const c_char,
        error: *mut *mut c_char,
    ) -> c_int;
    fn sandbox_free_error(error: *mut c_char);
    fn sandbox_check(pid: libc::pid_t, operation: *const c_char, kind: c_int, ...) -> c_int;
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTaskCreateFromSelf(allocator: *const c_void) -> *const c_void;
    fn SecTaskCopyValueForEntitlement(
        task: *const c_void,
        name: *const c_void,
        error: *mut *const c_void,
    ) -> *const c_void;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(
        allocator: *const c_void,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: bool,
    ) -> *const c_void;
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: *const c_void) -> bool;
    fn CFRelease(value: *const c_void);
}
unsafe extern "C" {
    fn csops(pid: libc::pid_t, operation: u32, address: *mut c_void, size: usize) -> c_int;
}

pub(super) fn enter() -> io::Result<()> {
    close_extra_descriptors()?;
    let mut flags: u32 = 0;
    if unsafe {
        csops(
            libc::getpid(),
            0,
            (&mut flags as *mut u32).cast(),
            size_of::<u32>(),
        )
    } != 0
    {
        return Err(super::syscall_error("csops(CS_OPS_STATUS)"));
    }
    if flags & 0x10001 != 0x10001 {
        return Err(io::Error::other(
            "APP_SANDBOX_SIGNATURE: valid hardened-runtime worker required",
        ));
    }
    signing_policy()?;
    super::ceilings()?;
    let profile = CString::new(include_str!("macos.sb")).expect("static profile");
    let mut error = std::ptr::null_mut();
    let parameters = [std::ptr::null()];
    let result = unsafe {
        sandbox_init_with_parameters(profile.as_ptr(), 0, parameters.as_ptr(), &mut error)
    };
    let failure = (result != 0).then(|| super::syscall_error("sandbox_init_with_parameters"));
    if !error.is_null() {
        // A compiler diagnostic may contain host paths. Report only a stable
        // failure and let the standalone drill identify profile compatibility.
        unsafe {
            sandbox_free_error(error);
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    if unsafe { sandbox_check(libc::getpid(), std::ptr::null(), 0) } != 1 {
        return Err(io::Error::other(
            "APP_SANDBOX_PROFILE: Seatbelt installation failed",
        ));
    }
    Ok(())
}

fn close_extra_descriptors() -> io::Result<()> {
    // Darwin has no close_range/closefrom. Enumerate only our own descriptors,
    // before any thread or signing API can create another one.
    let mut task = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let size = size_of::<libc::proc_taskinfo>() as c_int;
    let actual = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            task.as_mut_ptr().cast(),
            size,
        )
    };
    if actual != size {
        return Err(super::syscall_error("proc_pidinfo(PROC_PIDTASKINFO)"));
    }
    let threads = unsafe { task.assume_init() }.pti_threadnum;
    if threads != 1 {
        return Err(io::Error::other(format!(
            "APP_SANDBOX_ORDER: containment must precede threads (found {threads})"
        )));
    }
    let size = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDLISTFDS,
            0,
            std::ptr::null_mut(),
            0,
        )
    };
    if !(1..=1024 * 1024).contains(&size) {
        return Err(super::syscall_error("proc_pidinfo(PROC_PIDLISTFDS, size)"));
    }
    let count = size as usize / size_of::<libc::proc_fdinfo>() + 32;
    let mut descriptors: Vec<libc::proc_fdinfo> =
        (0..count).map(|_| unsafe { std::mem::zeroed() }).collect();
    let actual = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDLISTFDS,
            0,
            descriptors.as_mut_ptr().cast(),
            (count * size_of::<libc::proc_fdinfo>()) as c_int,
        )
    };
    if actual <= 0 {
        return Err(super::syscall_error("proc_pidinfo(PROC_PIDLISTFDS, list)"));
    }
    if actual as usize >= count * size_of::<libc::proc_fdinfo>() {
        return Err(io::Error::other(
            "APP_SANDBOX_FDS: descriptor enumeration incomplete",
        ));
    }
    for descriptor in &descriptors[..actual as usize / size_of::<libc::proc_fdinfo>()] {
        if descriptor.proc_fd > 4 && unsafe { libc::close(descriptor.proc_fd) } != 0 {
            return Err(super::syscall_error("close(inherited descriptor)"));
        }
    }
    Ok(())
}

fn signing_policy() -> io::Result<()> {
    let task = unsafe { SecTaskCreateFromSelf(std::ptr::null()) };
    if task.is_null() {
        return Err(io::Error::other(
            "APP_SANDBOX_SIGNATURE: signing information unavailable",
        ));
    }
    let entitled = |name: &str| -> bool {
        let key = unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                name.as_ptr(),
                name.len() as isize,
                0x08000100,
                false,
            )
        };
        if key.is_null() {
            return false;
        }
        let value = unsafe { SecTaskCopyValueForEntitlement(task, key, std::ptr::null_mut()) };
        unsafe {
            CFRelease(key);
        }
        if value.is_null() {
            return false;
        }
        let enabled =
            unsafe { CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value) };
        unsafe {
            CFRelease(value);
        }
        enabled
    };
    let valid = entitled("com.apple.security.cs.allow-jit")
        && ![
            "com.apple.security.cs.allow-unsigned-executable-memory",
            "com.apple.security.cs.disable-executable-page-protection",
            "com.apple.security.cs.disable-library-validation",
            "com.apple.security.cs.allow-dyld-environment-variables",
            "com.apple.security.get-task-allow",
            "get-task-allow",
        ]
        .iter()
        .any(|name| entitled(name));
    unsafe {
        CFRelease(task);
    }
    if !valid {
        return Err(io::Error::other(
            "APP_SANDBOX_SIGNATURE: only hardened JIT code memory is permitted",
        ));
    }
    Ok(())
}
