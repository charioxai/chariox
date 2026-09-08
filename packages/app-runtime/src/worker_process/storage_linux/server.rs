//! One root-owned helper loop. Connections retain leases; disconnect recovery
//! stays with this process and its durable journal, not an unowned cleanup task.
use super::{
    files, formatter,
    model::{Enrollment, Request},
    store::Store,
    wire::{self, Reader, Reply},
    Error, Result, CONFIG, SOCKET_ROOT,
};
use std::{
    ffi::{CString, OsStr},
    io::Read,
    os::{
        fd::AsRawFd,
        unix::{
            fs::MetadataExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}

pub(super) fn run(arguments: Vec<String>) -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(Error::Identity);
    }
    if arguments
        .first()
        .is_some_and(|value| value == "--format-child")
    {
        return formatter::child(&arguments[1..]);
    }
    if arguments == ["--prepare-managed-domain"] {
        return super::provision::managed(configuration()?);
    }
    if !arguments.is_empty() {
        return Err(Error::Invalid);
    }
    unsafe {
        libc::umask(0o077);
    }
    if unsafe { libc::setgroups(0, std::ptr::null()) } != 0 {
        return Err(Error::Identity);
    }
    let config = configuration()?;
    let uids = config
        .owners
        .iter()
        .map(|owner| owner.uid)
        .collect::<Vec<_>>();
    let mut store = Store::open(config)?;
    let socket_root = files::root_directory(Path::new(SOCKET_ROOT))?;
    if socket_root.0.metadata()?.mode() & 0o7777 != 0o711 {
        return Err(Error::Identity);
    }
    let mut listeners = Vec::new();
    for uid in uids {
        let name = format!("u-{uid}.sock");
        match crate::private_fs::entry_metadata(&socket_root, OsStr::new(&name)) {
            Ok(metadata) => {
                if metadata.st_mode & libc::S_IFMT != libc::S_IFSOCK || metadata.st_uid != uid {
                    return Err(Error::Identity);
                }
                socket_root.remove_file(OsStr::new(&name))?;
            }
            Err(crate::private_fs::FsError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let path = Path::new(SOCKET_ROOT).join(&name);
        let listener = UnixListener::bind(&path)?;
        let encoded = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        if unsafe { libc::chown(encoded.as_ptr(), uid, u32::MAX) } != 0
            || unsafe { libc::chmod(encoded.as_ptr(), 0o600) } != 0
        {
            return Err(Error::Io);
        }
        socket_root.sync()?;
        listener.set_nonblocking(true)?;
        listeners.push((uid, listener));
    }
    for signal in [libc::SIGTERM, libc::SIGINT] {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = stop as usize;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(Error::Io);
        }
    }
    // Type=notify makes helper initialization/recovery a real dependency of
    // kernel startup. No client races a merely spawned but unready daemon.
    if std::env::var("NOTIFY_SOCKET").ok().as_deref() != Some("/run/systemd/notify") {
        return Err(Error::Identity);
    }
    let notification = std::os::unix::net::UnixDatagram::unbound()?;
    notification.set_write_timeout(Some(Duration::from_secs(1)))?;
    if notification.send_to(b"READY=1", "/run/systemd/notify")? != 7 {
        return Err(Error::Io);
    }
    drop(notification);
    let mut clients = Vec::<Client>::new();
    let mut pending = Vec::<(u32, String)>::new();
    while !STOP.load(Ordering::Relaxed) {
        for (uid, listener) in &listeners {
            // One accept per listener/pass prevents an admitted UID from starving
            // already owned cleanup. Kernel backlog and live leases are bounded.
            if clients.len() + pending.len() >= 64 {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    if wire::peer(&stream).ok() != Some(*uid) || !store.enrolled(*uid) {
                        continue;
                    }
                    clients.push(Client {
                        uid: *uid,
                        stream,
                        reader: Reader::default(),
                        lease: None,
                        deadline: Instant::now() + Duration::from_secs(5),
                    });
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err(Error::Io),
            }
        }
        let mut index = 0;
        while index < clients.len() {
            if clients[index].step(&mut store).is_err() {
                let client = clients.swap_remove(index);
                if let Some(lease) = client.lease {
                    pending.push((client.uid, lease));
                }
            } else {
                index += 1;
            }
        }
        // A failed unmount/empty check remains an owned pending lease. Never
        // release accounting simply because the socket owner disappeared.
        if let Some((uid, lease)) = pending.first().cloned() {
            if store.release(uid, &lease, true).is_ok() {
                pending.remove(0);
            } else {
                pending.rotate_left(1);
            }
        }
        store.retry_pending();
        let mut polls = listeners
            .iter()
            .map(|(_, listener)| libc::pollfd {
                fd: listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .chain(clients.iter().map(|client| libc::pollfd {
                fd: client.stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            }))
            .collect::<Vec<_>>();
        if unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as _, 50) } < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            return Err(Error::Io);
        }
    }
    drop(clients);
    store.shutdown();
    Ok(())
}
struct Client {
    uid: u32,
    stream: UnixStream,
    reader: Reader,
    lease: Option<String>,
    deadline: Instant,
}
impl Client {
    fn step(&mut self, store: &mut Store) -> Result<()> {
        if self.lease.is_none() && Instant::now() >= self.deadline {
            return Err(Error::Busy);
        }
        let Some(bytes) = self.reader.next(&self.stream)? else {
            return Ok(());
        };
        self.reader = Reader::default();
        let request: Request = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        request.validate()?;
        match (&self.lease, request) {
            (None, request @ Request::Acquire { .. }) => match store.acquire(self.uid, request) {
                Ok(grant) => {
                    self.lease = Some(grant.lease.clone());
                    wire::send(
                        &mut self.stream,
                        &Reply {
                            status: "acquired".into(),
                            grant: Some(grant),
                        },
                    )?;
                }
                Err(error) => {
                    let _ = wire::send(&mut self.stream, &Reply::failed(error));
                    return Err(error);
                }
            },
            (Some(expected), Request::Release { lease }) if *expected == lease => {
                match store.release(self.uid, &lease, false) {
                    Ok(()) => {
                        self.lease = None;
                        let _ = wire::send(&mut self.stream, &Reply::released());
                        return Err(Error::Io); // close the consumed connection
                    }
                    Err(error) => {
                        wire::send(&mut self.stream, &Reply::failed(error))?;
                    }
                }
            }
            _ => return Err(Error::Identity),
        }
        Ok(())
    }
}

fn configuration() -> Result<Enrollment> {
    let config_path = Path::new(CONFIG);
    let config_parent = files::root_directory(config_path.parent().unwrap())?;
    let mut config_file = config_parent.read_file(config_path.file_name().unwrap(), false)?;
    files::root_owned(&config_file, false)?;
    if config_file.metadata()?.len() > 16384 {
        return Err(Error::Invalid);
    }
    let mut bytes = Vec::new();
    (&mut config_file).take(16385).read_to_end(&mut bytes)?;
    let config: Enrollment = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    config.validate()?;
    Ok(config)
}
