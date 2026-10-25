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
    /// MP-08/MP-10/MP-11: private kernel-resolved owned-window crop.
    #[serde(default)]
    window_region: Option<WindowRegion>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowRegion {
    #[serde(default)]
    id: Option<u32>,
    width: i32,
    height: i32,
    left: i32,
    top: i32,
}
impl Config {
    fn valid(&self) -> bool {
        if self.pid <= 1 || !self.pool.is_absolute() || self.desktop && self.window_region.is_some()
        {
            return false;
        }
        match &self.window_region {
            None => [(1280, 800), (2560, 1600), (1920, 1080)].contains(&(self.width, self.height)),
            Some(window) => {
                window.id.is_none_or(|id| id > 1)
                    && (2..=2560).contains(&self.width)
                    && (2..=1600).contains(&self.height)
                    && self.width % 2 == 0
                    && self.height % 2 == 0
                    && (1..=2560).contains(&window.width)
                    && (1..=2400).contains(&window.height)
                    && window.left >= 0
                    && window.top >= 0
                    && window.left <= window.width - self.width
                    && window.top <= window.height - self.height
            }
        }
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;
    // Protocol 530: the exact control lines kernel-browser-native.mjs and
    // kernel-browser-window-region.mjs write (NativeWorkerControl adds `id`).
    #[test]
    fn mp08_band_gesture_and_inventory_control_lines_parse() {
        for line in [
            r#"{"click":{"id":1,"point":[10,20]}}"#,
            r#"{"click":{"id":2,"point":[10,20],"button":3}}"#,
            r#"{"drag":{"id":3,"point":[10,20],"to":[300,20]}}"#,
            r#"{"windows":{"id":4}}"#,
        ] {
            assert!(serde_json::from_str::<Command>(line).is_ok(), "{line}");
        }
        assert!(matches!(
            serde_json::from_str::<Command>(r#"{"windows":{"id":4}}"#),
            Ok(Command::Windows { .. })
        ));
        // The worker's own bound owner is the only inventory owner.
        assert!(serde_json::from_str::<Command>(r#"{"windows":{"id":4,"pid":1}}"#).is_err());
    }
    #[test]
    fn mp11_region_config_cannot_escape_owned_window_or_widen_legacy_geometry() {
        let base = json!({"pid":42,"width":1280,"height":174,"pool":"/owned/pool",
            "window_region":{"width":1280,"height":974,"left":0,"top":0}});
        assert!(serde_json::from_value::<Config>(base.clone())
            .unwrap()
            .valid());
        for (key, value) in [
            ("id", 0),
            ("id", 1),
            ("left", -1),
            ("top", 801),
            ("width", 1279),
            ("height", 2410),
        ] {
            let mut bad = base.clone();
            bad["window_region"][key] = value.into();
            assert!(
                !serde_json::from_value::<Config>(bad).unwrap().valid(),
                "{key}"
            );
        }
        let mut legacy = base.clone();
        legacy.as_object_mut().unwrap().remove("window_region");
        assert!(!serde_json::from_value::<Config>(legacy).unwrap().valid());
        for (key, value) in [
            ("pid", json!(1)),
            ("height", json!(175)),
            ("pool", json!("relative")),
        ] {
            let mut bad = base.clone();
            bad[key] = value;
            assert!(
                !serde_json::from_value::<Config>(bad).unwrap().valid(),
                "{key}"
            );
        }
    }
}
#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Command {
    // MP-08/MP-10/MP-11: upstream fences cannot be inferred from raster
    // pixels. Count their fixed reasons at the native worker, never content.
    Reject {
        reject: Rejection,
    },
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
        wheel: Pointer,
    },
    Click {
        click: Pointer,
    },
    Drag {
        drag: Pointer,
    },
    Windows {
        windows: WindowsQuery,
    },
    Key {
        key: [u32; 2],
    },
    KeyChord {
        key_chord: [u32; 2],
    },
    Plans {
        plans: bool,
    },
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Rejection {
    StaleDocument,
    Epoch,
    RegionRevision,
    SerialSkipped,
    ShiftHold,
}
impl Rejection {
    fn index(self) -> usize {
        self as usize
    }
    fn label(self) -> &'static str {
        [
            "stale_document",
            "epoch",
            "region_revision",
            "serial_skipped",
            "shift_hold",
        ][self.index()]
    }
}
/// MP-08/MP-10/MP-11: pointer input is a request: the reply says whether the
/// owned window received it, so Node falls back to CDP on a refusal.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    id: u64,
    point: [i32; 2],
    #[serde(default)]
    notches: Option<[i32; 2]>,
    /// Protocol 530: 2 (middle) or 3 (right); absent means the primary button.
    #[serde(default)]
    button: Option<i32>,
    /// Protocol 530: a drag's end point.
    #[serde(default)]
    to: Option<[i32; 2]>,
}
/// Protocol 530: the bound owner's window inventory from the running worker.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowsQuery {
    id: u64,
}
fn admit_default() -> bool {
    true
}
const RASTER_SLOTS: usize = super::RASTER_SLOT_NAMES.len();
// MP-08/MP-10: damage-driven FramePace semantics described in the Selkies
// audit (pixelflux pace.rs). Independently implemented here; no upstream
// code copied. Input wakes retain their immediate readback admission.
const READBACK_MIN: Duration = Duration::from_millis(3);
pub(super) fn readback_due(dirty: bool, wake: bool, elapsed: Duration) -> bool {
    dirty && (wake || elapsed >= READBACK_MIN)
}
pub(super) fn readback_wait(dirty: bool, free_slot: bool, elapsed: Duration) -> i32 {
    if dirty && free_slot {
        READBACK_MIN
            .saturating_sub(elapsed)
            .as_micros()
            .div_ceil(1000) as i32
    } else if free_slot {
        // Upper bound only; XDamage/control readiness wakes poll sooner.
        16
    } else {
        // No slot can be overwritten. Release/control/XDamage still wake us.
        1000
    }
}
pub(super) fn epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.
}
pub(super) fn emit_to(
    out: &mut impl Write,
    header: Value,
    payload: &[u8],
    limit: usize,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(&header).map_err(|_| "MP-11: native header")?;
    if bytes.len() > 8192 || payload.is_empty() || payload.len() > limit {
        return Err("MP-11: native reply bound".into());
    }
    out.write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| out.write_all(&bytes))
        .and_then(|_| out.write_all(payload))
        .and_then(|_| out.flush())
        .map_err(|_| "MP-11: native reply pipe".into())
}
fn emit(header: Value, payload: &[u8]) -> Result<(), String> {
    emit_to(
        &mut std::io::stdout().lock(),
        header,
        payload,
        2560 * 1600 * 4,
    )
}
pub(super) fn reply(id: u64, value: Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(&value).map_err(|_| "MP-11: native reply")?;
    emit_to(
        &mut std::io::stdout().lock(),
        json!({"reply":id,"length":bytes.len()}),
        &bytes,
        8 * 1024 * 1024,
    )
}
pub(super) fn exact_reply_to(out: &mut impl Write, id: u64, value: Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(&value).map_err(|_| "MP-11: native reply")?;
    // MP-08/MP-10/MP-11: base64 repair bytes can exceed the raw raster's
    // ceiling. Only the exact worker emits this larger private reply.
    emit_to(
        out,
        json!({"reply":id,"length":bytes.len()}),
        &bytes,
        32 * 1024 * 1024,
    )
}
pub(super) fn exact_reply(id: u64, value: Value) -> Result<(), String> {
    exact_reply_to(&mut std::io::stdout().lock(), id, value)
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
    if !config.valid() {
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
    // MP-08/MP-10: the existing owned-desktop environment presents the root.
    let automatic = i32::from(std::env::var("CHARIOX_OWNED_VIRTUAL_DISPLAY").as_deref() == Ok("1"));
    super::probe_codec()?;
    // MP-08/MP-10: a viewer at the other density resizes the window just
    // before this start; X applies it asynchronously. Wait up to one second.
    let mut capture = Capture(std::ptr::null_mut());
    for attempt in 0..20 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        capture = Capture(unsafe {
            match &config.window_region {
                Some(window) if window.id.is_some() => ffi::cx_capture_open_selected_region(
                    config.pid as libc::c_ulong,
                    window.id.unwrap() as libc::c_ulong,
                    window.width,
                    window.height,
                    window.left,
                    window.top,
                    w,
                    h,
                ),
                Some(window) => ffi::cx_capture_open_region(
                    config.pid as libc::c_ulong,
                    window.width,
                    window.height,
                    window.left,
                    window.top,
                    w,
                    h,
                ),
                None => ffi::cx_capture_open_mode(owner as libc::c_ulong, w, h, automatic),
            }
        });
        if !capture.0.is_null() {
            break;
        }
    }
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
    let mut rejected = [0u64; 5];
    for reason in [
        Rejection::StaleDocument,
        Rejection::Epoch,
        Rejection::RegionRevision,
        Rejection::SerialSkipped,
        Rejection::ShiftHold,
    ] {
        eprintln!("MD-DISPLAY: native rejected {} 0", reason.label());
    }
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
    let (codec_tx, codec_rx) =
        std::sync::mpsc::sync_channel::<(Encode, raster::CodecLease, f64)>(4);
    let codec_sessions = sessions.clone();
    // A stable OS thread name permits CPU accounting without timing traces.
    std::thread::Builder::new()
        .name("cx-video-codec".into())
        .spawn(move || {
            let lock = || {
                codec_sessions
                    .lock()
                    .map_err(|_| "MP-11: native sessions poisoned".to_string())
            };
            while let Ok((q, lease, queued)) = codec_rx.recv() {
                let started = epoch();
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
                if result
                    .and_then(|mut value| {
                        // MP-08/MP-10/MP-11: private numeric queue timing only.
                        if let Some(spans) = value["timings"].as_array_mut() {
                            spans.push(json!(["native_codec_queue", queued, started]));
                        }
                        reply(q.id, value)
                    })
                    .is_err()
                {
                    std::process::exit(2);
                }
            }
        })
        .map_err(|_| "MD-DISPLAY: native codec worker unavailable")?;
    let mut bounds = [0, 0, w, h];
    loop {
        if unsafe { ffi::cx_capture_damage(capture.0) } != 0 {
            if !dirty {
                damage_at = epoch();
            }
            dirty = true;
        }
        let due = readback_due(dirty, wake.is_some(), last.elapsed());
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
        let timeout = readback_wait(dirty, slots.iter().any(|s| s.available()), last.elapsed());
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
                Command::Reject { reject } => {
                    let count = &mut rejected[reject.index()];
                    *count = count.saturating_add(1);
                    eprintln!("MD-DISPLAY: native rejected {} {}", reject.label(), count);
                }
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
                // MP-08/MP-10/MP-11: device-pixel point and [dx, dy] notches;
                // refused (covered, not owned) wheels/clicks are reported.
                Command::Wheel { wheel: p } => {
                    let [dx, dy] = p.notches.ok_or("MP-11: native wheel")?;
                    let [x, y] = p.point;
                    let wheeled = unsafe { ffi::cx_capture_wheel(capture.0, x, y, dx, dy) } == 0;
                    if wheeled {
                        wake = Some(epoch());
                    }
                    reply(p.id, json!({ "wheeled": wheeled }))?;
                }
                Command::Click { click: p } => {
                    let [x, y] = p.point;
                    let button = p.button.unwrap_or(1);
                    let clicked = unsafe { ffi::cx_capture_button(capture.0, x, y, button) } == 0;
                    if clicked {
                        wake = Some(epoch());
                    }
                    reply(p.id, json!({ "clicked": clicked }))?;
                }
                Command::Drag { drag: p } => {
                    let ([x, y], [tx, ty]) = (p.point, p.to.ok_or("MP-11: native drag")?);
                    let dragged = unsafe { ffi::cx_capture_drag(capture.0, x, y, tx, ty) } == 0;
                    if dragged {
                        wake = Some(epoch());
                    }
                    reply(p.id, json!({ "dragged": dragged }))?;
                }
                // A failed read refuses this request only; the capture keeps running.
                Command::Windows { windows: q } => match owner_windows(owner) {
                    Ok(rows) => reply(q.id, json!({ "windows": rows }))?,
                    Err(_) => reply(q.id, json!({ "error": "MP-11: native window inventory" }))?,
                },
                // MP-08/MP-10: [keysym, shift] one key press/release.
                Command::Key {
                    key: [keysym, shift],
                } => {
                    if unsafe { ffi::cx_capture_key(capture.0, keysym.into(), shift as i32) } != 0 {
                        return Err("MP-11: native key refused".into());
                    }
                    wake = Some(epoch());
                }
                Command::KeyChord {
                    key_chord: [keysym, modifiers],
                } => {
                    if modifiers > 15
                        || !((0x20..=0x7e).contains(&keysym) || (0xff08..=0xffff).contains(&keysym))
                        || unsafe {
                            ffi::cx_capture_key_modifiers(
                                capture.0,
                                keysym.into(),
                                modifiers as i32,
                            )
                        } != 0
                    {
                        return Err("MP-11: native key chord refused".into());
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
                    match codec_tx.try_send((q, slot.codec_lease(), epoch())) {
                        Ok(()) => {}
                        Err(std::sync::mpsc::TrySendError::Full((q, _, _))) => {
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
                        // MP-08/MP-10/MP-11: PNG work must not stall input or
                        // readback. The plan owns its masked patch pixels;
                        // the existing bounded worker finishes all exact jobs.
                        if !exact.submit(id, plan)? {
                            reply(id, json!({"busy": true}))?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// MP-08/MP-10/MP-11: one-shot numeric inventory, private owned-display ABI.
pub(super) fn inventory() -> Result<(), String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Query {
        pid: u32,
    }
    let mut line = String::new();
    use std::io::Read;
    std::io::stdin()
        .take(8193)
        .read_to_string(&mut line)
        .map_err(|_| "MP-11: native window query")?;
    if line.len() > 8192 {
        return Err("MP-11: native window query bound".into());
    }
    let q: Query = serde_json::from_str(&line).map_err(|_| "MP-11: native window query")?;
    serde_json::to_writer(std::io::stdout().lock(), &owner_windows(q.pid)?)
        .map_err(|_| "MP-11: native window inventory reply".into())
}
fn owner_windows(pid: u32) -> Result<Vec<Value>, String> {
    if pid <= 1 {
        return Err("MP-11: native window owner".into());
    }
    let mut data = [0i64; 32 * 13];
    let count = unsafe { ffi::cx_capture_inventory(pid as libc::c_ulong, data.as_mut_ptr(), 32) };
    if count < 0 {
        return Err("MP-11: native window inventory".into());
    }
    Ok(data.chunks_exact(13).take(count as usize).map(|r|json!({"id":r[0],"width":r[1],"height":r[2],"x":r[3],"y":r[4],"visible":r[5]==1,"override_redirect":r[6]==1,"transient_for":r[7],"focused":r[8]==1,"frame_insets":[r[9],r[10],r[11],r[12]]})).collect())
}
