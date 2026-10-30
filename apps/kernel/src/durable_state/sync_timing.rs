//! Measure SQLite's actual sync calls without changing its durability policy.
use std::cell::Cell;
use std::ffi::{c_char, c_int};
use std::sync::OnceLock;
use std::time::Instant;

use rusqlite::ffi;
use rusqlite::{Connection, OpenFlags};

thread_local! { static SYNC_US: Cell<u64> = const { Cell::new(0) }; }

pub(super) fn sync_us() -> u64 {
    SYNC_US.with(Cell::get)
}

#[repr(C)]
struct TimedVfs {
    vfs: ffi::sqlite3_vfs,
    original: *mut ffi::sqlite3_vfs,
}

#[repr(C)]
struct TimedMethods {
    methods: ffi::sqlite3_io_methods,
    original: *const ffi::sqlite3_io_methods,
}

const NAME: &[u8] = b"chariox-durable-timing\0";

pub(super) fn open(path: &std::path::Path) -> Result<Connection, String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED.get_or_init(register).clone()?;
    Connection::open_with_flags_and_vfs(path, OpenFlags::default(), "chariox-durable-timing")
        .map_err(|error| error.to_string())
}

fn register() -> Result<(), String> {
    // SAFETY: SQLite owns the default VFS for the process lifetime. The registered
    // wrapper is also process-lived and preserves every callback and app-data
    // field except xOpen; xOpen delegates to the original VFS before wrapping I/O.
    unsafe {
        let original = ffi::sqlite3_vfs_find(std::ptr::null());
        if original.is_null() {
            return Err("SQLite has no default filesystem".into());
        }
        let mut vfs = std::ptr::read(original);
        vfs.pNext = std::ptr::null_mut();
        vfs.zName = NAME.as_ptr().cast();
        vfs.xOpen = Some(open_file);
        let wrapper = Box::into_raw(Box::new(TimedVfs { vfs, original }));
        let result = ffi::sqlite3_vfs_register(&mut (*wrapper).vfs, 0);
        if result != ffi::SQLITE_OK {
            drop(Box::from_raw(wrapper));
            return Err(format!(
                "SQLite timing filesystem registration failed: {result}"
            ));
        }
    }
    Ok(())
}

unsafe extern "C" fn open_file(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    // SAFETY: SQLite passes our registered VFS and a buffer of the unchanged
    // default szOsFile. The original xOpen initializes the original file layout.
    let original = unsafe { (*(vfs.cast::<TimedVfs>())).original };
    let result = unsafe { ((*original).xOpen.unwrap())(original, name, file, flags, out_flags) };
    if result == ffi::SQLITE_OK {
        // The table stays alive until this exact file's xClose. Every callback
        // besides xSync/xClose is copied unchanged, including optional v2/v3 I/O.
        unsafe {
            let original = (*file).pMethods;
            if !original.is_null() {
                let mut methods = std::ptr::read(original);
                methods.xSync = Some(sync_file);
                methods.xClose = Some(close_file);
                let wrapper = Box::into_raw(Box::new(TimedMethods { methods, original }));
                (*file).pMethods = &(*wrapper).methods;
            }
        }
    }
    result
}

unsafe extern "C" fn sync_file(file: *mut ffi::sqlite3_file, flags: c_int) -> c_int {
    // SAFETY: xOpen installed TimedMethods on this file; SQLite serializes I/O
    // on a connection. No callbacks borrow the wrapper across xClose.
    let original = unsafe { (*((*file).pMethods.cast::<TimedMethods>())).original };
    let started = Instant::now();
    let result = unsafe { ((*original).xSync.unwrap())(file, flags) };
    let elapsed = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    SYNC_US.with(|total| total.set(total.get().saturating_add(elapsed)));
    result
}

unsafe extern "C" fn close_file(file: *mut ffi::sqlite3_file) -> c_int {
    // SAFETY: take ownership before the original close can clear pMethods.
    // Restore the native table for the filesystem's own close implementation.
    let wrapper = unsafe { Box::from_raw((*file).pMethods as *mut TimedMethods) };
    unsafe {
        (*file).pMethods = wrapper.original;
        ((*wrapper.original).xClose.unwrap())(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_sync_timing_preserves_commit_and_reopen() {
        let root =
            std::env::temp_dir().join(format!("sync-timing-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("state.db");
        {
            let before_write = sync_us();
            let db = open(&path).unwrap();
            db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE example(value); INSERT INTO example VALUES(7);").unwrap();
            assert!(
                sync_us() > before_write,
                "FULL commits must exercise the timed xSync callback"
            );
            let before = sync_us();
            assert_eq!(
                db.query_row("SELECT value FROM example", [], |row| row.get::<_, i32>(0))
                    .unwrap(),
                7
            );
            assert_eq!(
                sync_us(),
                before,
                "read operations must not be counted as sync waits"
            );
        }
        let db = Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("SELECT value FROM example", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            7
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}
