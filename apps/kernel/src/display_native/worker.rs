//! MP-08/MP-10/MP-11: private control, bounded leases and packet ownership.
use super::{
    exact::ExactWorker,
    ffi::{self, Capture},
    raster::{self, Slot},
    sessions::{Encode, Exact, Sessions},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Write,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    pid: u32,
    width: i32,
    height: i32,
    pool: PathBuf,
    /// MP-08/MP-11: read the kernel-owned desktop root instead of an owned
    /// browser window; the kernel masks every frame before encode.
    #[serde(default)]
    desktop: bool,
}
#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Command {
    Encode {
        encode: Encode,
    },
    Exact {
        exact: Exact,
    },
    Retire {
        retire: String,
    },
    Commit {
        commit: String,
        serial: u64,
        /// MP-08/MP-10: lossless scroll commits keep the session overlay only.
        #[serde(default = "admit_default")]
        admit: bool,
    },
    Delivered {
        delivered: String,
        revision: u64,
    },
    Release {
        release: usize,
        serial: u64,
    },
    Wake {
        wake: bool,
    },
    Refresh {
        refresh: bool,
    },
    Wheel {
        wheel: [i32; 4],
    },
    Click {
        click: [i32; 2],
    },
    Key {
        key: [u32; 2],
    },
    Plans {
        plans: bool,
    },
}
fn admit_default() -> bool {
    true
}
const RASTER_SLOTS: usize = 6;
pub(super) fn epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.
}
fn emit(header: Value, payload: &[u8]) -> Result<(), String> {
    let bytes = serde_json::to_vec(&header).map_err(|_| "MP-11: native header")?;
    if bytes.len() > 16384 || payload.is_empty() || payload.len() > 2560 * 1600 * 4 {
        return Err("MP-11: native reply bound".into());
    }
    let mut out = std::io::stdout().lock();
    out.write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| out.write_all(&bytes))
        .and_then(|_| out.write_all(payload))
        .and_then(|_| out.flush())
        .map_err(|_| "MP-11: native reply pipe".into())
}
pub(super) fn reply(id: u64, value: Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(&value).map_err(|_| "MP-11: native reply")?;
    emit(json!({"reply":id,"length":bytes.len()}), &bytes)
}
fn read_config() -> Result<Config, String> {
    // Unbuffered first line: do not consume an immediately following control.
    let mut bytes = Vec::new();
    let mut byte = 0u8;
    while bytes.len() < 8192 {
        if unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) } != 1 {
            return Err("MP-11: native config pipe".into());
        }
        if byte == b'\n' {
            return serde_json::from_slice(&bytes).map_err(|_| "MP-11: native config".into());
        }
        bytes.push(byte);
    }
    Err("MP-11: native config bound".into())
}
pub(super) fn run() -> Result<(), String> {
    let config = read_config()?;
    let (w, h) = (config.width, config.height);
    if config.pid <= 1
        || ![(1280, 800), (2560, 1600), (1920, 1080)].contains(&(w, h))
        || !config.pool.is_absolute()
    {
        return Err("MP-11: native admission".into());
    }
    let info =
        std::fs::symlink_metadata(&config.pool).map_err(|_| "MP-11: native pool metadata")?;
    if !info.is_dir() || info.uid() != unsafe { libc::geteuid() } || info.mode() & 0o077 != 0 {
        return Err("MP-11: native pool owner".into());
    }
    let root = config
        .pool
        .parent()
        .ok_or("MP-11: native packet root")?
        .to_path_buf();
    let root_info = std::fs::symlink_metadata(&root).map_err(|_| "MP-11: native packet root")?;
    if !root_info.is_dir() || root_info.uid() != info.uid() || root_info.mode() & 0o077 != 0 {
        return Err("MP-11: native root owner".into());
    }
    let owner = if config.desktop { 0 } else { config.pid };
    let capture = Capture(unsafe { ffi::cx_capture_open(owner as libc::c_ulong, w, h) });
    if capture.0.is_null() {
        return Err("MP-10: native capture unavailable".into());
    }
    // MP-08/MP-10: Node holds the latest and a pending readback while the
    // codec/exact threads lease another; three slots starved input-echo
    // readbacks for 30-60 ms on hosted typing. Keep in sync with Node's pool.
    let mut slots = (0..RASTER_SLOTS)
        .map(|i| Slot::create(&config.pool, i, (w * h * 4) as usize))
        .collect::<Result<Vec<_>, _>>()?;
    let mut control = Vec::new();
    let mut serial = 0u64;
    let mut admitted = 0u64;
    let exact = ExactWorker::new();
    let mut dirty = true;
    let mut refresh = true;
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut wake = None;
    let mut damage_at = epoch();
    // MP-08/MP-10: video encoding runs on its own thread so readback continues
    // during x264; capture damage queues while that thread holds the sessions.
    let sessions = std::sync::Arc::new(std::sync::Mutex::new(Sessions::new(w, h, root)));
    let mut pending_rows = 0u8;
    let (codec_tx, codec_rx) = std::sync::mpsc::sync_channel::<(Encode, raster::CodecLease)>(4);
    let codec_sessions = sessions.clone();
    std::thread::spawn(move || {
        let lock = || {
            codec_sessions
                .lock()
                .map_err(|_| "MP-11: native sessions poisoned".to_string())
        };
        while let Ok((q, lease)) = codec_rx.recv() {
            // x264 runs between two short critical sections.
            let result = lock().and_then(|mut s| s.begin_encode(&q)).and_then(|job| {
                let outcome = Sessions::run_encode(&job, lease.pixels());
                let mut sessions = lock()?;
                match outcome {
                    Ok(outcome) => sessions.finish_encode(&q, job, outcome),
                    Err(error) => Err(error),
                }
            });
            // Encoder failures stay fatal for the worker, as on the capture thread.
            if result.and_then(|value| reply(q.id, value)).is_err() {
                std::process::exit(2);
            }
        }
    });
    let mut bounds = [0, 0, w, h];
    loop {
        if unsafe { ffi::cx_capture_damage(capture.0) } != 0 {
            if !dirty {
                damage_at = epoch();
            }
            dirty = true;
        }
        let due = dirty && (wake.is_some() || last.elapsed() >= Duration::from_millis(16));
        if due {
            if let Some(slot) = slots.iter_mut().find(|s| s.available()) {
                let at = epoch();
                last = Instant::now();
                let changed =
                    unsafe { ffi::cx_capture_read(capture.0, slot.pixels, bounds.as_mut_ptr()) };
                if changed < 0 {
                    return Err("MP-11: native window retired".into());
                }
                dirty = false;
                if changed > 0 || refresh {
                    if changed == 0 {
                        bounds = [0, 0, w, h];
                    }
                    pending_rows |= Sessions::damage_rows(h, bounds);
                    if let Ok(mut s) = sessions.try_lock() {
                        s.damage(pending_rows);
                        pending_rows = 0;
                    }
                    serial = serial
                        .checked_add(1)
                        .ok_or("MP-11: native serial overflow")?;
                    slot.serial = Some(serial);
                    slot.bounds = bounds;
                    let mut tile_bounds = [0i32; 512];
                    let count =
                        unsafe { ffi::cx_capture_tiles(capture.0, tile_bounds.as_mut_ptr()) };
                    slot.tiles = if changed > 0 && (1..=128).contains(&count) {
                        Some(
                            tile_bounds[..count as usize * 4]
                                .chunks_exact(4)
                                .map(|r| [r[0], r[1], r[2], r[3]])
                                .collect(),
                        )
                    } else {
                        None
                    };
                    let tile_headers = slot.tiles.clone();
                    let adjacent_count = unsafe {
                        ffi::cx_capture_adjacent_tiles(capture.0, tile_bounds.as_mut_ptr())
                    };
                    slot.adjacent = if changed > 0 && (1..=128).contains(&adjacent_count) {
                        Some(
                            tile_bounds[..adjacent_count as usize * 4]
                                .chunks_exact(4)
                                .map(|r| [r[0], r[1], r[2], r[3]])
                                .collect(),
                        )
                    } else {
                        None
                    };
                    let adjacent_headers = slot.adjacent.clone();
                    slot.shift = raster::Shift::read(capture.0);
                    slot.base = admitted;
                    let shift_header = slot.shift.clone();
                    let motion_height = unsafe { ffi::cx_capture_motion_height(capture.0) };
                    let index = slots.iter().position(|s| s.serial == Some(serial)).unwrap();
                    let mut cpu = [0f64; 3];
                    unsafe {
                        ffi::cx_capture_cpu(capture.0, cpu.as_mut_ptr());
                    }
                    emit(
                        json!({"identical":changed == 0,"shift_adjacent":shift_header,"motion_height":motion_height,"adjacent_damage_tiles":adjacent_headers,"damage_tiles":tile_headers,"native_cpu":cpu,"slot":index,"width":w,"height":h,"length":1,"serial":serial,"base_serial":admitted,"patch":null,"signature":format!("{serial:016x}"),"captured_ms":at,"capture_ms":epoch()-at,"damage":bounds,"damage_ready_ms":damage_at,"native_read_ms":epoch(),"input_wake_ms":wake}),
                        &[0],
                    )?;
                    refresh = false;
                }
                wake = None;
            }
        }
        let timeout = if dirty && slots.iter().any(|s| s.available()) {
            16u128.saturating_sub(last.elapsed().as_millis()).min(16) as i32
        } else {
            1000
        };
        let mut fds = [
            libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: unsafe { ffi::cx_capture_fd(capture.0) },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) } < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err("MP-11: native poll".into());
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP) == 0 {
            continue;
        }
        let mut bytes = [0u8; 8192];
        let read = unsafe { libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) };
        if read == 0 {
            break;
        }
        if read < 0 {
            return Err("MP-11: native control pipe".into());
        }
        control.extend_from_slice(&bytes[..read as usize]);
        if control.len() > 8 * 1024 * 1024 {
            return Err("MP-11: native control bound".into());
        }
        while let Some(end) = control.iter().position(|b| *b == b'\n') {
            let command: Command =
                serde_json::from_slice(&control[..end]).map_err(|_| "MP-11: native control")?;
            control.drain(..=end);
            let mut locked = || -> Result<std::sync::MutexGuard<'_, Sessions>, String> {
                let mut s = sessions
                    .lock()
                    .map_err(|_| "MP-11: native sessions poisoned")?;
                s.damage(pending_rows);
                pending_rows = 0;
                Ok(s)
            };
            match command {
                Command::Wake { wake: true } => wake = Some(epoch()),
                Command::Refresh { refresh: true } => {
                    dirty = true;
                    refresh = true;
                    wake = Some(epoch());
                }
                Command::Wake { wake: false } | Command::Refresh { refresh: false } => {
                    return Err("MP-11: native wake".into())
                }
                Command::Release { release, serial } => {
                    let slot = slots.get_mut(release).ok_or("MP-11: native slot")?;
                    if slot.serial != Some(serial) {
                        return Err("MP-11: native lease mismatch".into());
                    }
                    slot.serial = None;
                }
                Command::Retire { retire } => locked()?.retire(&retire),
                Command::Plans { plans } => unsafe {
                    ffi::cx_capture_plans(capture.0, plans as i32)
                },
                // MP-08/MP-10: [x, y, dx, dy] in device pixels and notches.
                Command::Wheel {
                    wheel: [x, y, dx, dy],
                } => {
                    if unsafe { ffi::cx_capture_wheel(capture.0, x, y, dx, dy) } != 0 {
                        return Err("MP-11: native wheel refused".into());
                    }
                    wake = Some(epoch());
                }
                // MP-08/MP-10: [x, y] primary click in device pixels.
                Command::Click { click: [x, y] } => {
                    if unsafe { ffi::cx_capture_click(capture.0, x, y) } != 0 {
                        return Err("MP-11: native click refused".into());
                    }
                    wake = Some(epoch());
                }
                // MP-08/MP-10: [keysym, shift] one key press/release.
                Command::Key {
                    key: [keysym, shift],
                } => {
                    if unsafe { ffi::cx_capture_key(capture.0, keysym.into(), shift as i32) } != 0 {
                        return Err("MP-11: native key refused".into());
                    }
                    wake = Some(epoch());
                }
                Command::Commit {
                    commit,
                    serial: committed,
                    admit,
                } => {
                    // MP-08/MP-10: admission (adjacency witnesses) stays bound to
                    // the latest readback; the session overlay follows every commit.
                    if locked()?.commit(&commit, committed, committed == serial)
                        && committed == serial
                        && admit
                    {
                        if let Some(slot) = slots.iter().find(|s| s.serial == Some(committed)) {
                            unsafe {
                                ffi::cx_capture_admit(capture.0, slot.pixels);
                            }
                            admitted = committed;
                        }
                    }
                }
                Command::Delivered {
                    delivered,
                    revision,
                } => locked()?.delivered(&delivered, revision),
                Command::Encode { encode: q } => {
                    let slot = slots
                        .iter()
                        .find(|s| s.serial == Some(q.serial))
                        .ok_or("MP-11: native encode lease")?;
                    // MP-10 (#933 review 7): a full codec queue refuses this
                    // encode softly (Node skips one sample); never fatal.
                    match codec_tx.try_send((q, slot.codec_lease())) {
                        Ok(()) => {}
                        Err(std::sync::mpsc::TrySendError::Full((q, _))) => {
                            reply(q.id, json!({"busy": true}))?
                        }
                        Err(_) => return Err("MP-11: native codec queue unavailable".into()),
                    }
                }
                Command::Exact { exact: q } => {
                    let id = q.id;
                    let slot = slots
                        .iter()
                        .find(|s| s.serial == Some(q.serial))
                        .ok_or("MP-11: native exact lease")?;
                    // MP-08/MP-10: scroll residuals encode synchronously while
                    // this leased slot cannot be released or overwritten.
                    // A plan that no longer matches the committed canvas is an
                    // ordinary refusal; Node falls back to video. Never fatal.
                    if q.shift.is_some() {
                        let refused =
                            |reason: String| json!({"shift_refused": true, "reason": reason});
                        match locked()?.shift(q, slot) {
                            Ok(job) => {
                                if !matches!(exact.submit_job(id, job), Ok(true)) {
                                    reply(id, refused("MP-10: native exact queue busy".into()))?;
                                }
                            }
                            Err(reason) => reply(id, refused(reason))?,
                        }
                    } else {
                        let plan = locked()?.exact(q, slot)?;
                        // Input echo patches are bounded and latency-critical:
                        // finish them here rather than behind queued repairs.
                        if plan.patch {
                            let value = plan.finish().unwrap_or_else(
                                |_| json!({"error":"MP-11: exact preparation failed"}),
                            );
                            reply(id, value)?;
                        } else if !exact.submit(id, plan)? {
                            reply(id, json!({"busy": true}))?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
