//! Only root-created image descriptors can reach LOOP_CONFIGURE. Device numbers
//! are observed kernel allocation results, never request fields or detach authority.
use super::{files, model::Identity, Error, Result};
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    },
    time::{Duration, Instant},
};

// Linux UAPI include/uapi/linux/loop.h: identical fixed-width ABI on x64/arm64.
#[repr(C)]
#[derive(Clone, Copy)]
struct Info {
    device: u64,
    inode: u64,
    rdevice: u64,
    offset: u64,
    sizelimit: u64,
    number: u32,
    encrypt_type: u32,
    encrypt_key_size: u32,
    flags: u32,
    file_name: [u8; 64],
    crypt_name: [u8; 64],
    encrypt_key: [u8; 32],
    init: [u64; 2],
}
#[repr(C)]
struct Configure {
    fd: u32,
    block_size: u32,
    info: Info,
    reserved: [u64; 8],
}
const _: () = assert!(std::mem::size_of::<Info>() == 232);
const _: () = assert!(std::mem::size_of::<Configure>() == 304);
const CLEAR: libc::c_ulong = 0x4c01;
const GET_STATUS: libc::c_ulong = 0x4c05;
const CONFIGURE: libc::c_ulong = 0x4c0a;
const GET_FREE: libc::c_ulong = 0x4c82;
const AUTOCLEAR: u32 = 4;

pub(super) struct Device {
    pub file: File,
    pub minor: u32,
    image: Identity,
    capacity: u64,
}
impl Device {
    pub fn attach(image: &File, capacity: u64) -> Result<Self> {
        files::root_owned(image, false)?;
        if ![super::DATA_BYTES, super::TMP_BYTES].contains(&capacity)
            || image.metadata()?.len() != capacity
        {
            return Err(Error::Identity);
        }
        let identity = files::identity(image)?;
        if let Some(existing) = Self::find(&identity, capacity)? {
            return Ok(existing);
        }
        let control = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/dev/loop-control")?;
        let metadata = control.metadata()?;
        if !metadata.file_type().is_char_device()
            || metadata.uid() != 0
            || libc::major(metadata.rdev()) != 10
            || libc::minor(metadata.rdev()) != 237
        {
            return Err(Error::Identity);
        }
        // GET_FREE is advisory. CONFIGURE claims the selected device atomically;
        // another loop user winning the race causes a small bounded retry.
        for _ in 0..16 {
            let minor = unsafe { libc::ioctl(control.as_raw_fd(), GET_FREE) };
            if !(0..4096).contains(&minor) {
                return Err(Error::Capacity);
            }
            let file = open(minor as u32)?;
            let mut config: Configure = unsafe { std::mem::zeroed() };
            config.fd = image.as_raw_fd() as u32;
            config.block_size = 4096;
            config.info.sizelimit = capacity;
            config.info.flags = AUTOCLEAR;
            if unsafe { libc::ioctl(file.as_raw_fd(), CONFIGURE, &config) } != 0 {
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::EBUSY) {
                    continue;
                }
                return Err(Error::Io);
            }
            let device = Self {
                file,
                minor: minor as u32,
                image: identity.clone(),
                capacity,
            };
            device.verify()?;
            return Ok(device);
        }
        Err(Error::Busy)
    }

    /// Recovery scans a bounded kernel device inventory for the held image
    /// inode. A remembered /dev/loopN alone is never trusted, even for cleanup.
    pub fn find(image: &Identity, capacity: u64) -> Result<Option<Self>> {
        let mut found = None;
        for (count, entry) in std::fs::read_dir("/sys/block")?.enumerate() {
            if count >= 8192 {
                return Err(Error::Capacity);
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix("loop")) else {
                continue;
            };
            let minor = suffix.parse::<u32>().map_err(|_| Error::Identity)?;
            if minor >= 4096 || minor.to_string() != suffix {
                return Err(Error::Identity);
            }
            let file = open(minor)?;
            let Some(info) = status(&file)? else {
                continue;
            };
            if info.device != image.device || info.inode != image.inode {
                continue;
            }
            if found.is_some() {
                return Err(Error::Identity);
            }
            let device = Self {
                file,
                minor,
                image: image.clone(),
                capacity,
            };
            device.verify()?;
            found = Some(device);
        }
        Ok(found)
    }
    pub fn verify(&self) -> Result<()> {
        let info = status(&self.file)?.ok_or(Error::Identity)?;
        if info.number != self.minor
            || info.device != self.image.device
            || info.inode != self.image.inode
            || info.offset != 0
            || info.sizelimit != self.capacity
            || info.flags != AUTOCLEAR
            || info.encrypt_type != 0
            || info.encrypt_key_size != 0
        {
            return Err(Error::Identity);
        }
        Ok(())
    }
    pub fn device_id(&self) -> u64 {
        libc::makedev(7, self.minor)
    }
    pub fn descriptor_path(&self) -> String {
        format!("/proc/self/fd/{}", self.file.as_raw_fd())
    }

    /// Caller has already verified and unmounted the exact owned mount. Linux
    /// may defer LOOP_CLR_FD while another reference exists: do not acknowledge
    /// reclamation until a fresh bounded scan proves this image is unbound.
    pub fn detach(self) -> Result<()> {
        self.verify()?;
        if unsafe { libc::ioctl(self.file.as_raw_fd(), CLEAR) } != 0 {
            return Err(Error::Io);
        }
        let Self {
            file,
            image,
            capacity,
            ..
        } = self;
        drop(file);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Self::find(&image, capacity)?.is_none() {
                return Ok(());
            }
            // LOOP_CLR_FD may defer actual teardown while another kernel or
            // udev reference drains. Retain ownership and inspect the same
            // backing identity; do not detach another loop or acknowledge early.
            if Instant::now() >= deadline {
                return Err(Error::RecoveryRequired);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn open(minor: u32) -> Result<File> {
    if minor >= 4096 {
        return Err(Error::Identity);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(format!("/dev/loop{minor}"))?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_block_device()
        || metadata.uid() != 0
        || libc::major(metadata.rdev()) != 7
        || libc::minor(metadata.rdev()) != minor
    {
        return Err(Error::Identity);
    }
    Ok(file)
}
fn status(file: &File) -> Result<Option<Info>> {
    let mut info: Info = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(file.as_raw_fd(), GET_STATUS, &mut info) } != 0 {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENXIO) {
            Ok(None)
        } else {
            Err(Error::Io)
        };
    }
    Ok(Some(info))
}
