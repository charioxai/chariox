//! L1: the bound owner's window inventory, off the worker loop. One persistent X connection walks
//! the (unchanged) inventory on its own thread; a watcher thread reports the events that can
//! change a row with one coalesced `windows_changed` header, re-armed by the next walk.
use super::{ffi, worker};
use serde_json::json;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

struct Connection(*mut c_void);
// Every use is serialized by the mutex around it.
unsafe impl Send for Connection {}

pub(super) struct Inventory(mpsc::Sender<u64>);

impl Inventory {
    pub(super) fn start(owner: u32) -> Result<Self, String> {
        if owner <= 1 {
            return Err("MP-11: native window owner".into());
        }
        let raw = unsafe { ffi::cx_inventory_open(owner as libc::c_ulong) };
        if raw.is_null() {
            return Err("MP-11: native window inventory".into());
        }
        let fd = unsafe { ffi::cx_inventory_fd(raw) };
        let connection = Arc::new(Mutex::new(Connection(raw)));
        let announced = Arc::new(AtomicBool::new(false));
        {
            let (connection, announced) = (connection.clone(), announced.clone());
            std::thread::spawn(move || watch(fd, &connection, &announced));
        }
        let (tx, rx) = mpsc::channel::<u64>();
        std::thread::spawn(move || {
            for id in rx {
                announced.store(false, Ordering::SeqCst);
                let mut data = [0i64; 32 * 13];
                let count = match connection.lock() {
                    Ok(c) => unsafe { ffi::cx_inventory_read(c.0, data.as_mut_ptr(), 32) },
                    Err(_) => -1,
                };
                let value = match worker::window_rows(&data, count) {
                    Ok(rows) => json!({ "windows": rows }),
                    Err(_) => json!({ "error": "MP-11: native window inventory" }),
                };
                if worker::reply(id, value).is_err() {
                    return;
                }
            }
        });
        Ok(Self(tx))
    }
    pub(super) fn ask(&self, id: u64) {
        let _ = self.0.send(id);
    }
}

fn watch(fd: i32, connection: &Mutex<Connection>, announced: &AtomicBool) {
    loop {
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut p, 1, -1) } < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        if p.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            return;
        }
        let changed = match connection.lock() {
            Ok(c) => unsafe { ffi::cx_inventory_changed(c.0) },
            Err(_) => return,
        };
        if changed > 0 && !announced.swap(true, Ordering::SeqCst) {
            let header = json!({"windows_changed":worker::epoch(),"length":1});
            if worker::emit(header, &[0]).is_err() {
                return;
            }
        }
    }
}
