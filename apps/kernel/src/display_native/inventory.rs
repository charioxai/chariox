//! L1: the bound owner's window inventory, off the worker loop. One thread owns a persistent X
//! connection: it walks the (unchanged) inventory on request and otherwise waits for the events that
//! can change a row, reported with one coalesced `windows_changed` header re-armed by the next walk.
//! Walking and waiting on one thread means events a walk reads into Xlib's queue still wake it.
use super::{ffi, worker};
use serde_json::json;
use std::ffi::c_void;
use std::sync::mpsc;

struct Connection(*mut c_void);
// Owned by the inventory thread alone.
unsafe impl Send for Connection {}

pub(super) struct Inventory {
    requests: mpsc::Sender<u64>,
    wake: i32,
}

impl Inventory {
    pub(super) fn start(owner: u32) -> Result<Self, String> {
        if owner <= 1 {
            return Err("MP-11: native window owner".into());
        }
        let wake = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if wake < 0 {
            return Err("MP-11: native window inventory".into());
        }
        let raw = unsafe { ffi::cx_inventory_open(owner as libc::c_ulong) };
        if raw.is_null() {
            unsafe { libc::close(wake) };
            return Err("MP-11: native window inventory".into());
        }
        let (requests, rx) = mpsc::channel::<u64>();
        let connection = Connection(raw);
        std::thread::spawn(move || run(&connection, wake, &rx));
        Ok(Self { requests, wake })
    }
    pub(super) fn ask(&self, id: u64) {
        if self.requests.send(id).is_ok() {
            let one = 1u64;
            unsafe { libc::write(self.wake, (&one as *const u64).cast(), 8) };
        }
    }
}

fn run(connection: &Connection, wake: i32, requests: &mpsc::Receiver<u64>) {
    let mut announced = false;
    loop {
        while let Ok(id) = requests.try_recv() {
            announced = false;
            let mut data = [0i64; 32 * 13];
            let count = unsafe { ffi::cx_inventory_read(connection.0, data.as_mut_ptr(), 32) };
            let value = match worker::window_rows(&data, count) {
                Ok(rows) => json!({ "windows": rows }),
                Err(_) => json!({ "error": "MP-11: native window inventory" }),
            };
            if worker::reply(id, value).is_err() {
                return;
            }
        }
        match unsafe { ffi::cx_inventory_wait(connection.0, wake, -1) } {
            1 if !announced => {
                announced = true;
                let header = json!({"windows_changed":worker::epoch(),"length":1});
                if worker::emit(header, &[0]).is_err() {
                    return;
                }
            }
            2 => {
                let mut count = 0u64;
                unsafe { libc::read(wake, (&mut count as *mut u64).cast(), 8) };
            }
            1 => {}
            _ => return,
        }
    }
}
