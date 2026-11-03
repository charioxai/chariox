//! MP-08 / MP-10 / MP-11: synchronous writes to the pinned, preallocated journal.
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

#[repr(C, align(4096))]
#[derive(Clone)]
struct Page([u8; 4096]);

// RWF_DSYNC supplies data-integrity completion; O_DIRECT alone is insufficient.
// The record length/checksum exclude trailing alignment bytes, so readers and
// torn-write recovery retain the existing two-slot format on every platform.
// Unsupported kernel/filesystem alignment returns to buffered write + sync_data.
pub(super) fn write(file: &File, record: &[u8], offset: u64) -> io::Result<bool> {
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_DIRECT) } < 0 {
        let error = io::Error::last_os_error();
        return if unsupported(&error) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    let mut pages = vec![Page([0; 4096]); record.len().div_ceil(4096)];
    for (page, bytes) in pages.iter_mut().zip(record.chunks(4096)) {
        page.0[..bytes.len()].copy_from_slice(bytes);
    }
    let len = pages.len() * 4096;
    let iov = libc::iovec {
        iov_base: pages.as_ptr().cast_mut().cast(),
        iov_len: len,
    };
    loop {
        // SAFETY: the File owns fd; aligned contiguous pages remain live for
        // this synchronous call, and the bounded journal offset fits off_t.
        let written =
            unsafe { libc::pwritev2(fd, &iov, 1, offset as libc::off_t, libc::RWF_DSYNC) };
        if written == len as isize {
            return Ok(true);
        }
        if written >= 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short journal write",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if !unsupported(&error) {
            return Err(error);
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } < 0 {
            return Err(io::Error::last_os_error());
        }
        return Ok(false);
    }
}

fn unsupported(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EINVAL | libc::EOPNOTSUPP | libc::ENOSYS)
    )
}
