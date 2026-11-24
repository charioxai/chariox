//! Raw output crossing a reload belongs in memory until the job can scrub its next complete line.
use std::io;
#[cfg(not(target_os = "linux"))]
use std::os::fd::AsRawFd as _;
use std::os::fd::{FromRawFd as _, OwnedFd};

use zeroize::Zeroizing;

#[cfg(target_os = "linux")]
pub(super) fn write(bytes: &[u8]) -> io::Result<OwnedFd> {
    let fd = super::memory_file_for_next_program(c"chariox-handover-job", bytes)?;
    // The memory file was just created for this job.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
pub(super) fn read(fd: OwnedFd) -> io::Result<Zeroizing<Vec<u8>>> {
    use std::io::Read as _;
    let mut body = Zeroizing::new(Vec::new());
    std::fs::File::from(fd).read_to_end(&mut body)?;
    Ok(body)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn write(bytes: &[u8]) -> io::Result<OwnedFd> {
    let name = std::ffi::CString::new(format!("/cj{:024x}", rand::random::<u128>() >> 32))
        .expect("generated shared memory name");
    // shm_open leaves FD_CLOEXEC set; the reload passes only descriptors it owns.
    let fd = unsafe {
        libc::shm_open(
            name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::shm_unlink(name.as_ptr()) } != 0
        || unsafe { libc::ftruncate(fd.as_raw_fd(), bytes.len() as libc::off_t) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let mapping = Mapping::open(&fd, bytes.len(), libc::PROT_READ | libc::PROT_WRITE)?;
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapping.address.cast(), bytes.len()) };
    Ok(fd)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read(fd: OwnedFd) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let len = usize::try_from(unsafe { stat.assume_init() }.st_size).map_err(io::Error::other)?;
    let mapping = Mapping::open(&fd, len, libc::PROT_READ)?;
    Ok(Zeroizing::new(
        unsafe { std::slice::from_raw_parts(mapping.address.cast(), len) }.to_vec(),
    ))
}

#[cfg(not(target_os = "linux"))]
struct Mapping {
    address: *mut libc::c_void,
    len: usize,
}

#[cfg(not(target_os = "linux"))]
impl Mapping {
    fn open(fd: &OwnedFd, len: usize, protection: i32) -> io::Result<Self> {
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                protection,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { address, len })
    }
}

#[cfg(not(target_os = "linux"))]
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.address, self.len) };
    }
}
