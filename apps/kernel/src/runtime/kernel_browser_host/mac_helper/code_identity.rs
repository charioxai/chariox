//! macOS M1: socket peer audit token, code requirement and start identity.
use std::ffi::c_void;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::ptr::null;

type CFTypeRef = *const c_void;
#[repr(C)]
struct Opaque([u8; 0]);

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFTypeDictionaryKeyCallBacks: Opaque;
    static kCFTypeDictionaryValueCallBacks: Opaque;
    fn CFDataCreate(allocator: CFTypeRef, bytes: *const u8, length: isize) -> CFTypeRef;
    fn CFStringCreateWithBytes(
        allocator: CFTypeRef,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> CFTypeRef;
    fn CFDictionaryCreate(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const Opaque,
        value_callbacks: *const Opaque,
    ) -> CFTypeRef;
    fn CFRelease(value: CFTypeRef);
}
#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecGuestAttributeAudit: CFTypeRef;
    fn SecCodeCopyGuestWithAttributes(
        host: CFTypeRef,
        attributes: CFTypeRef,
        flags: u32,
        guest: *mut CFTypeRef,
    ) -> i32;
    fn SecRequirementCreateWithString(text: CFTypeRef, flags: u32, out: *mut CFTypeRef) -> i32;
    fn SecCodeCheckValidity(code: CFTypeRef, flags: u32, requirement: CFTypeRef) -> i32;
}
const UTF8: u32 = 0x0800_0100;
const SOL_LOCAL: libc::c_int = 0;
const LOCAL_PEERTOKEN: libc::c_int = 6;

struct Owned(CFTypeRef);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

/// Kernel-reported audit token of the connected peer, never a claimed PID.
pub(super) struct Peer([u32; 8]);
impl Peer {
    pub(super) fn of(stream: &UnixStream) -> Result<Self, String> {
        let mut token = [0u32; 8];
        let mut length = std::mem::size_of_val(&token) as libc::socklen_t;
        let status = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                SOL_LOCAL,
                LOCAL_PEERTOKEN,
                token.as_mut_ptr().cast(),
                &mut length,
            )
        };
        if status != 0 || length as usize != std::mem::size_of_val(&token) {
            return Err("MP-11: Computer helper peer identity unavailable".into());
        }
        Ok(Self(token))
    }
    pub(super) fn euid(&self) -> u32 {
        self.0[1]
    }
    pub(super) fn pid(&self) -> i32 {
        self.0[5] as i32
    }
    /// The live peer's dynamic code satisfies `requirement`.
    pub(super) fn satisfies(&self, requirement: &str) -> bool {
        unsafe {
            let token = Owned(CFDataCreate(null(), self.0.as_ptr().cast(), 32));
            let text = Owned(CFStringCreateWithBytes(
                null(),
                requirement.as_ptr(),
                requirement.len() as isize,
                UTF8,
                0,
            ));
            if token.0.is_null() || text.0.is_null() {
                return false;
            }
            let attributes = Owned(CFDictionaryCreate(
                null(),
                &kSecGuestAttributeAudit,
                &token.0,
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            ));
            let (mut code, mut rule) = (Owned(null()), Owned(null()));
            !attributes.0.is_null()
                && SecCodeCopyGuestWithAttributes(null(), attributes.0, 0, &mut code.0) == 0
                && SecRequirementCreateWithString(text.0, 0, &mut rule.0) == 0
                && SecCodeCheckValidity(code.0, 0, rule.0) == 0
        }
    }
}

/// Start time of a live, non-zombie process; `None` once it has exited.
pub(super) fn started(pid: i32) -> Option<(u64, u64)> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (read == size && info.pbi_status != libc::SZOMB)
        .then_some((info.pbi_start_tvsec, info.pbi_start_tvusec))
}
