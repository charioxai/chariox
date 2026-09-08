//! Kernel-side ownership of one authenticated helper lease. This private type
//! belongs inside the worker resource domain; Apps cannot construct it.
use super::{
    files, model,
    store::Grant,
    wire::{self, Reply},
    Error, Result, DATA_BYTES, ROOT, SOCKET_ROOT, TMP_BYTES,
};
use crate::private_fs::Dir;
use serde::Serialize;
use std::{
    ffi::OsStr,
    os::{
        fd::AsRawFd,
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
};

pub(super) struct Lease {
    stream: UnixStream,
    grant: Grant,
    pub data: Option<Dir>,
    pub temporary: Option<Dir>,
    path: PathBuf,
    released: bool,
}
#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Operation<'a> {
    Acquire {
        owner: &'a str,
        installation: &'a str,
        generation: u64,
        cgroup_leaf: &'a str,
    },
    Release {
        lease: &'a str,
    },
}
impl Lease {
    pub fn acquire(
        owner: &str,
        installation: &str,
        generation: u64,
        cgroup_leaf: &str,
    ) -> Result<Self> {
        let uid = unsafe { libc::geteuid() };
        if uid == 0 {
            return Err(Error::Identity);
        }
        let request = model::Request::Acquire {
            owner: owner.into(),
            installation: installation.into(),
            generation,
            cgroup_leaf: cgroup_leaf.into(),
        };
        request.validate()?;
        let name = model::installation_name(owner, installation)?;
        let socket_root = files::search_root_directory(Path::new(SOCKET_ROOT))?;
        let socket_name = format!("u-{uid}.sock");
        let metadata = crate::private_fs::entry_metadata(&socket_root, OsStr::new(&socket_name))?;
        if metadata.st_mode & libc::S_IFMT != libc::S_IFSOCK
            || metadata.st_uid != uid
            || metadata.st_mode & 0o7777 != 0o600
        {
            return Err(Error::Identity);
        }
        let mut stream = UnixStream::connect(Path::new(SOCKET_ROOT).join(socket_name))?;
        stream.set_nonblocking(true)?;
        if wire::peer(&stream)? != 0 {
            return Err(Error::Identity);
        }
        wire::send(
            &mut stream,
            &Operation::Acquire {
                owner,
                installation,
                generation,
                cgroup_leaf,
            },
        )?;
        let reply: Reply = wire::receive(&stream, 150)?;
        if reply.status != "acquired" {
            return Err(Error::RecoveryRequired);
        }
        let grant = reply.grant.ok_or(Error::Identity)?;
        let path = Path::new(ROOT).join(format!("u-{uid}")).join(name);
        if grant.data != path.join("data").to_string_lossy()
            || grant.temporary != path.join("tmp").to_string_lossy()
            || grant.data_bytes != DATA_BYTES
            || grant.temporary_bytes != TMP_BYTES
        {
            return Err(Error::Identity);
        }
        model::hex(&grant.lease, 32)?;
        let parent = files::search_root_directory(&path)?;
        let data = parent.child(OsStr::new("data"))?;
        let temporary = parent.child(OsStr::new("tmp"))?;
        verify(&data, grant.data_mount_id, DATA_BYTES, uid)?;
        verify(&temporary, grant.temporary_mount_id, TMP_BYTES, uid)?;
        if data.0.metadata()?.dev() == temporary.0.metadata()?.dev() {
            return Err(Error::Identity);
        }
        Ok(Self {
            stream,
            grant,
            data: Some(data),
            temporary: Some(temporary),
            path,
            released: false,
        })
    }
    pub fn data_path(&self) -> PathBuf {
        self.path.join("data")
    }
    pub fn temporary_path(&self) -> PathBuf {
        self.path.join("tmp")
    }
    /// Call after the worker cgroup is empty and every broker/worker directory
    /// pin has drained, but before dropping/removing that cgroup. Held mount FDs
    /// prevent an ordinary unmount; we do not hide ordering bugs with force.
    pub fn release(&mut self) -> Result<()> {
        if self.released {
            return Ok(());
        }
        self.data.take();
        self.temporary.take();
        wire::send(
            &mut self.stream,
            &Operation::Release {
                lease: &self.grant.lease,
            },
        )?;
        let reply: Reply = wire::receive(&self.stream, 10)?;
        if reply.status != "released" || reply.grant.is_some() {
            return Err(Error::RecoveryRequired);
        }
        self.released = true;
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.release();
        // EOF transfers remaining cleanup to the already owning helper. Its
        // pending journal stays durable if a mount or process cannot be reaped.
    }
}
fn verify(dir: &Dir, mount_id: u64, capacity: u64, uid: u32) -> Result<()> {
    let metadata = dir.0.metadata()?;
    if mount_id == 0 || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700 {
        return Err(Error::Identity);
    }
    let mut fs = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(dir.0.as_raw_fd(), fs.as_mut_ptr()) } != 0 {
        return Err(Error::Io);
    }
    let fs = unsafe { fs.assume_init() };
    let observed_flags = files::mount_flags(&dir.0)?;
    let flags = libc::ST_NOEXEC | libc::ST_NODEV | libc::ST_NOSUID;
    if fs.f_type != 0xef53
        || observed_flags & flags != flags
        || observed_flags & libc::ST_RDONLY != 0
        || fs.f_blocks.saturating_mul(fs.f_bsize as u64) > capacity
    {
        return Err(Error::Identity);
    }
    let mut stat = std::mem::MaybeUninit::<libc::statx>::zeroed();
    if unsafe {
        libc::statx(
            dir.0.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            stat.as_mut_ptr(),
        )
    } != 0
    {
        return Err(Error::Io);
    }
    let stat = unsafe { stat.assume_init() };
    if stat.stx_mask & libc::STATX_MNT_ID == 0 || stat.stx_mnt_id != mount_id {
        return Err(Error::Identity);
    }
    Ok(())
}
