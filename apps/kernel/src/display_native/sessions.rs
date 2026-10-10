//! MP-08/MP-10/MP-11: native codec sessions, delivered references and exact repairs.
use super::{
    exact::ExactPlan,
    ffi::{self, Codec, Rect, RowResult},
    raster::{self, Region, Slot},
    worker::epoch,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap, fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf,
};
fn stripe_default() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Encode {
    pub id: u64,
    pub encoder: String,
    pub serial: u64,
    pub bitrate: i32,
    pub reset: Value,
    pub regions: Vec<Region>,
    #[serde(default = "stripe_default")]
    pub stripes: bool,
    /// MP-08/MP-10: measured CPU contention fallback (reduced motion geometry).
    #[serde(default)]
    pub reduced: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Exact {
    pub id: u64,
    pub encoder: String,
    pub serial: u64,
    pub regions: Vec<Region>,
    pub patch: bool,
    #[serde(default)]
    pub adjacent: bool,
    #[serde(default)]
    pub repair_only: bool,
    /// MP-08/MP-10: "adjacent" (previous readback) or "overlay" (the
    /// committed exact canvas, planned now).
    #[serde(default)]
    pub shift: Option<String>,
    /// MP-08/MP-10: the delivered serial an "overlay" plan must be relative to.
    #[serde(default)]
    pub base: Option<u64>,
    /// MP-08/MP-10: lossless effort (0..=100); Node raises it when the link is busy.
    #[serde(default)]
    pub effort: Option<i32>,
}
/// MP-08/MP-10: the canvas a delivered exact frame leaves on the client: a
/// full raster, or patch tiles over the committed canvas of serial `base`.
enum Prepared {
    Full(Vec<u8>),
    Patch {
        base: u64,
        tiles: Vec<([i32; 4], Vec<u8>)>,
    },
}
pub(super) struct EncodeJob {
    codec: Codec,
    regions: Vec<Rect>,
    resets: u32,
    started: f64,
}
pub(super) enum EncodeOutcome {
    Dropped,
    Rows(Vec<(RowResult, Vec<u8>)>),
}
struct Session {
    /// Absent while the codec thread is encoding with it.
    codec: Option<Codec>,
    bitrate: i32,
    stripes: bool,
    reduced: bool,
    regions: Vec<Rect>,
    dirty: u8,
    exact: bool,
    revision: u64,
    delivered: u64,
    video_rows: u8,
    overlay: Option<Vec<u8>>,
    prepared: Option<(u64, Prepared)>,
    committed: u64,
    retired: bool,
}
fn session_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err("MP-11: native encoder binding".into());
    }
    Ok(())
}
/// MP-08/MP-10/MP-11: private packet file = big-endian u32 header length,
/// JSON segment headers (each with `length`), then the raw segment bytes.
/// The kernel binds these headers to the frame before projecting raw bytes.
pub(super) fn packet(
    root: &PathBuf,
    headers: &[Value],
    segments: &[&[u8]],
) -> Result<Value, String> {
    let header = serde_json::to_vec(headers).map_err(|_| "MP-11: native packet JSON")?;
    let length = 4 + header.len() + segments.iter().map(|s| s.len()).sum::<usize>();
    if headers.len() != segments.len()
        || segments.iter().any(|s| s.is_empty())
        || length > 1024 * 1024
    {
        return Err("MP-11: native packet bound".into());
    }
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&header);
    for segment in segments {
        bytes.extend_from_slice(segment);
    }
    let name = format!("{:032x}.json", rand::random::<u128>());
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join(&name))
        .map_err(|_| "MP-11: native packet open")?;
    file.write_all(&bytes)
        .map_err(|_| "MP-11: native packet write")?;
    Ok(json!({"name":name,"length":bytes.len()}))
}
fn clips(
    pixels: &[u8],
    w: i32,
    h: i32,
    bounds: [i32; 4],
    block: i32,
    rows: u8,
    session: Option<&Session>,
) -> Vec<[i32; 4]> {
    let mut output = Vec::new();
    for y in ((bounds[1] / block * block)..bounds[3]).step_by(block as usize) {
        let height = block.min(h - y);
        let touches = (0..8).any(|r| {
            rows & (1 << r) != 0
                && y < 2 * ((h / 2 * (r + 1)) / 8)
                && y + height > 2 * ((h / 2 * r) / 8)
        });
        if !touches {
            continue;
        }
        for x in ((bounds[0] / block * block)..bounds[2]).step_by(block as usize) {
            let width = block.min(w - x);
            let mut clip = [x, y, x + width, y + height];
            if let Some((s, codec)) = session
                .filter(|s| s.delivered == s.revision)
                .and_then(|s| s.codec.as_ref().map(|codec| (s, codec)))
            {
                unsafe {
                    ffi::cx_codec_repair_bounds(
                        codec.0,
                        pixels.as_ptr(),
                        s.overlay.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                        s.video_rows as u32,
                        x,
                        y,
                        width,
                        height,
                        clip.as_mut_ptr(),
                    );
                }
                if clip[0] >= clip[2] || clip[1] >= clip[3] {
                    continue;
                }
            }
            output.push(clip);
        }
    }
    output
}
pub(super) struct Sessions {
    sessions: HashMap<String, Session>,
    revision: u64,
    w: i32,
    h: i32,
    root: PathBuf,
}
impl Sessions {
    pub fn new(w: i32, h: i32, root: PathBuf) -> Self {
        Self {
            sessions: HashMap::new(),
            revision: 0,
            w,
            h,
            root,
        }
    }
    /// A session whose codec is out for an encode retires once that encode
    /// replies, preserving the single-thread command order.
    pub fn retire(&mut self, name: &str) {
        match self.sessions.get_mut(name) {
            Some(s) if s.codec.is_none() => s.retired = true,
            _ => {
                self.sessions.remove(name);
            }
        }
    }
    /// Codec row bands touched by a readback's damage bounds.
    pub fn damage_rows(h: i32, bounds: [i32; 4]) -> u8 {
        (0..8).fold(0, |rows, r| {
            if bounds[1] < 2 * ((h / 2 * (r + 1)) / 8) && bounds[3] > 2 * ((h / 2 * r) / 8) {
                rows | 1 << r
            } else {
                rows
            }
        })
    }
    pub fn damage(&mut self, rows: u8) {
        for s in self.sessions.values_mut() {
            s.dirty |= rows;
        }
    }
    /// MP-08/MP-10: the overlay always becomes the delivered exact canvas. A
    /// commit older than the latest readback keeps every row dirty, because
    /// damage from newer readbacks was recorded before this commit arrived.
    pub fn commit(&mut self, name: &str, serial: u64, latest: bool) -> bool {
        if let Some(s) = self.sessions.get_mut(name) {
            if s.prepared.as_ref().is_some_and(|(id, _)| *id == serial) {
                match s.prepared.take().unwrap().1 {
                    Prepared::Full(pixels) => s.overlay = Some(pixels),
                    // Patch tiles update the committed canvas they were cut
                    // against; any other overlay becomes unknown.
                    Prepared::Patch { base, tiles } => {
                        match s.overlay.as_mut().filter(|_| s.committed == base) {
                            Some(overlay) => {
                                let stride = self.w as usize * 4;
                                for ([x, y, w, h], bytes) in &tiles {
                                    for row in 0..*h as usize {
                                        let at = (*y as usize + row) * stride + *x as usize * 4;
                                        overlay[at..at + *w as usize * 4].copy_from_slice(
                                            &bytes[row * *w as usize * 4..][..*w as usize * 4],
                                        );
                                    }
                                }
                            }
                            None => s.overlay = None,
                        }
                    }
                }
                s.video_rows = 0;
                s.dirty = if latest { 0 } else { 255 };
                s.exact = true;
                s.committed = serial;
                return true;
            }
        }
        false
    }
    pub fn delivered(&mut self, name: &str, revision: u64) {
        if let Some(s) = self.sessions.get_mut(name) {
            if revision <= s.revision {
                s.delivered = s.delivered.max(revision);
            }
        }
    }
    /// MP-08/MP-10: validate the request and take the session codec out, so
    /// x264 runs without holding the sessions lock (input patches proceed).
    pub fn begin_encode(&mut self, q: &Encode) -> Result<EncodeJob, String> {
        let (w, h) = (self.w, self.h);
        let started = epoch();
        session_name(&q.encoder)?;
        if !(500000..=64000000).contains(&q.bitrate) {
            return Err("MP-11: native bitrate".into());
        }
        let regions = raster::regions(&q.regions, w, h)?;
        let mut resets = if q.reset == true {
            255
        } else if q.reset == false {
            0
        } else {
            q.reset
                .as_array()
                .ok_or("MP-11: native reset")?
                .iter()
                .try_fold(0u32, |v, r| {
                    let r = r
                        .as_u64()
                        .filter(|r| *r < 8)
                        .ok_or("MP-11: native reset row")?;
                    Ok::<_, String>(v | 1 << r)
                })?
        };
        let sessions = &mut self.sessions;
        if sessions.get(&q.encoder).is_some_and(|s| {
            s.regions != regions || s.stripes != q.stripes || s.reduced != q.reduced
        }) {
            sessions.remove(&q.encoder);
        }
        // MP-08/MP-10: a rate change retunes the live x264 rows (no IDR);
        // hardware rows cannot and reopen.
        if let Some(s) = sessions.get_mut(&q.encoder).filter(|s| s.bitrate != q.bitrate) {
            let retuned = s
                .codec
                .as_ref()
                .is_some_and(|codec| unsafe { ffi::cx_codec_rate(codec.0, q.bitrate) } == 0);
            if retuned {
                s.bitrate = q.bitrate;
            } else {
                sessions.remove(&q.encoder);
            }
        }
        if !sessions.contains_key(&q.encoder) {
            if sessions.len() >= 8 {
                return Err("MP-11: native encoder count".into());
            }
            let codec = Codec(unsafe {
                ffi::cx_codec_open(
                    w,
                    h,
                    q.bitrate,
                    if q.stripes { 8 } else { 1 },
                    q.reduced as i32,
                )
            });
            if codec.0.is_null() {
                return Err("MP-10: native codec unavailable".into());
            }
            sessions.insert(
                q.encoder.clone(),
                Session {
                    codec: Some(codec),
                    bitrate: q.bitrate,
                    stripes: q.stripes,
                    reduced: q.reduced,
                    regions: regions.clone(),
                    dirty: 255,
                    exact: false,
                    revision: 0,
                    delivered: 0,
                    video_rows: 0,
                    overlay: None,
                    prepared: None,
                    committed: 0,
                    retired: false,
                },
            );
            resets = 255;
        }
        let codec = sessions
            .get_mut(&q.encoder)
            .unwrap()
            .codec
            .take()
            .ok_or("MP-11: native codec busy")?;
        Ok(EncodeJob {
            codec,
            regions,
            resets,
            started,
        })
    }
    /// Run x264 for a begun job (no lock held); returns owned row packets.
    pub fn run_encode(job: &EncodeJob, pixels: *const u8) -> Result<EncodeOutcome, String> {
        let mut results = [RowResult::default(); 8];
        let count = unsafe {
            ffi::cx_codec_encode(
                job.codec.0,
                pixels,
                job.resets,
                job.regions.as_ptr(),
                job.regions.len(),
                results.as_mut_ptr(),
            )
        };
        if count == -2 {
            return Ok(EncodeOutcome::Dropped);
        }
        if !(0..=8).contains(&count) {
            return Err("MP-11: native encode unavailable".into());
        }
        let mut rows = Vec::new();
        for r in &results[..count as usize] {
            if r.bytes.is_null() || r.length == 0 || r.length > 1024 * 1024 {
                return Err("MP-11: native row bound".into());
            }
            let data = unsafe { std::slice::from_raw_parts(r.bytes, r.length) }.to_vec();
            rows.push((*r, data));
        }
        Ok(EncodeOutcome::Rows(rows))
    }
    /// Return the codec to its session (unless retired meanwhile) and reply.
    pub fn finish_encode(
        &mut self,
        q: &Encode,
        job: EncodeJob,
        outcome: EncodeOutcome,
    ) -> Result<Value, String> {
        let EncodeJob { codec, started, .. } = job;
        let backend = |codec: &Codec| {
            let kind = unsafe { ffi::cx_codec_backend(codec.0) };
            let backend = if kind == 1 {
                "native-vaapi"
            } else if unsafe { ffi::cx_codec_openh264(codec.0) } == 1 {
                "native-openh264"
            } else {
                "native-x264"
            };
            json!({"backend":backend,"hardware_fallback":kind == 2,"hardware_diagnostic":unsafe {std::ffi::CStr::from_ptr(ffi::cx_codec_diagnostic(codec.0))}.to_string_lossy(),"converter":"libyuv"})
        };
        let mut reply = backend(&codec);
        let reduced = unsafe { ffi::cx_codec_reduced(codec.0) } == 1;
        let mut cpu = [0f64; 6];
        unsafe {
            ffi::cx_codec_cpu(codec.0, cpu.as_mut_ptr());
        }
        let root = self.root.clone();
        let codec_revision = &mut self.revision;
        let session = self
            .sessions
            .get_mut(&q.encoder)
            .filter(|s| s.codec.is_none())
            .ok_or("MP-11: native encoder session")?;
        session.codec = Some(codec);
        let retired = session.retired;
        let rows = match outcome {
            EncodeOutcome::Dropped => {
                session.dirty = 255;
                session.exact = false;
                // MP-10: a protected packet rejection must retain the actual
                // hardware init outcome even when no video bytes are admitted.
                reply["dropped"] = true.into();
                if retired {
                    self.sessions.remove(&q.encoder);
                }
                return Ok(reply);
            }
            EncodeOutcome::Rows(rows) => rows,
        };
        let mut headers = Vec::new();
        for (r, _) in &rows {
            headers.push(json!({"row":r.row,"y":r.y,"height":r.height,"codec":"avc1.420033","key":r.key!=0,"sequence":r.sequence,"reference_sequence":if r.key!=0 {None}else{Some(r.reference)},"length":r.length}));
            session.dirty |= if q.stripes { 1 << r.row } else { 255 };
            session.video_rows |= if q.stripes { 1 << r.row } else { 255 };
        }
        if !rows.is_empty() {
            *codec_revision = codec_revision
                .checked_add(1)
                .ok_or("MP-11: native codec revision")?;
            session.revision = *codec_revision;
        }
        let encoded_at = epoch();
        let mut spans = vec![json!(["native_codec", started, encoded_at])];
        for (stage, duration) in [
            "native_cpu_mask_guard",
            "native_cpu_compare",
            "native_cpu_convert",
            "native_cpu_encode",
            "native_cpu_output_guard",
            "native_cpu_reference_copy",
        ]
        .into_iter()
        .zip(cpu)
        {
            spans.push(json!([stage, encoded_at - duration, encoded_at]));
        }
        let descriptor = if rows.is_empty() {
            None
        } else {
            let segments: Vec<&[u8]> = rows.iter().map(|(_, data)| data.as_slice()).collect();
            Some(packet(&root, &headers, &segments)?)
        };
        spans.push(json!(["codec_packetize", encoded_at, epoch()]));
        let revision = session.revision;
        if retired {
            self.sessions.remove(&q.encoder);
        }
        for (key, value) in [
            ("stripes", json!(headers)),
            ("packet", json!(descriptor)),
            ("workers", json!(1)),
            ("timings", json!(spans)),
            ("whole", json!(!q.stripes)),
            ("revision", json!(revision)),
            ("reduced", json!(reduced)),
        ] {
            reply[key] = value;
        }
        Ok(reply)
    }
    /// MP-08/MP-10: lossless scroll frame. Moves and residual rectangles were
    /// proved byte-exact at capture; only unprotected rasters are eligible.
    pub fn shift(&mut self, q: Exact, slot: &Slot) -> Result<super::exact::Job, String> {
        let started = epoch();
        session_name(&q.encoder)?;
        let session = self
            .sessions
            .get_mut(&q.encoder)
            .filter(|s| s.exact && s.regions.is_empty())
            .ok_or("MP-11: native shift base")?;
        let planned;
        let plan = match q.shift.as_deref() {
            Some("adjacent") => slot.shift.as_ref(),
            Some("overlay") => {
                // The overlay is the client's exact canvas only for its commit.
                let overlay = session
                    .overlay
                    .as_ref()
                    .filter(|_| q.base == Some(session.committed))
                    .ok_or("MP-11: native shift overlay")?;
                planned = raster::Shift::plan(slot.bytes(), overlay, self.w, self.h);
                planned.as_ref()
            }
            _ => return Err("MP-11: native shift kind".into()),
        }
        .ok_or("MP-11: native shift unavailable")?;
        if !q.regions.is_empty() {
            return Err("MP-11: protected shift refused".into());
        }
        // Copy only the residual rectangles here; the job encodes them in
        // parallel off the capture thread, so readback continues meanwhile.
        let pixels = slot.bytes();
        let stride = self.w as usize * 4;
        let rects: Vec<([i32; 4], Vec<u8>)> = plan
            .dirty
            .iter()
            .map(|&[x, y, w, h]| {
                let mut bytes = Vec::with_capacity((w * h * 4) as usize);
                for row in y..y + h {
                    let at = row as usize * stride + x as usize * 4;
                    bytes.extend_from_slice(&pixels[at..at + w as usize * 4]);
                }
                ([x, y, w, h], bytes)
            })
            .collect();
        session.prepared = Some((q.serial, Prepared::Full(pixels.to_vec())));
        let moves: Vec<[i32; 5]> = plan
            .moves
            .iter()
            .map(|[x, y, w, h]| [*x, *y, *w, *h, plan.dy])
            .collect();
        let effort = q.effort.unwrap_or(50);
        if !(0..=100).contains(&effort) {
            return Err("MP-11: native shift effort".into());
        }
        let (root, width, height, revision) = (self.root.clone(), self.w, self.h, session.revision);
        Ok(Box::new(move || {
            let encode = |rects: &[([i32; 4], Vec<u8>)]| {
                rects
                    .iter()
                    .map(|([_, _, w, h], bytes)| {
                        raster::webp(bytes, *w as usize * 4, [0, 0, *w, *h], 1, effort)
                    })
                    .collect::<Result<Vec<_>, _>>()
            };
            // Small residuals encode inline; large ones use up to four threads.
            let area: i32 = rects.iter().map(|([_, _, w, h], _)| w * h).sum();
            let part = if area < 65536 {
                rects.len().max(1)
            } else {
                rects.len().div_ceil(4).max(1)
            };
            let segments = std::thread::scope(|scope| {
                let workers: Vec<_> = rects
                    .chunks(part)
                    .map(|chunk| scope.spawn(move || encode(chunk)))
                    .collect();
                workers.into_iter().try_fold(Vec::new(), |mut all, worker| {
                    all.extend(
                        worker
                            .join()
                            .map_err(|_| "MP-10: lossless worker".to_string())??,
                    );
                    Ok::<_, String>(all)
                })
            });
            let result = segments.and_then(|segments| {
                let tiles: Vec<Value> = rects
                    .iter()
                    .zip(&segments)
                    .map(|(([x, y, w, h], _), data)| json!({"x":x,"y":y,"width":w,"height":h,"format":"webp","length":data.len()}))
                    .collect();
                let descriptor = if segments.is_empty() {
                    None
                } else {
                    let slices: Vec<&[u8]> = segments.iter().map(Vec::as_slice).collect();
                    Some(packet(&root, &tiles, &slices)?)
                };
                Ok(json!({"width":width,"height":height,"native_exact":true,"native_tiles":tiles,"moves":moves,"native_packet":descriptor,"native_revision":revision,"timings":[["native_shift_prepare",started,epoch()]]}))
            });
            result.unwrap_or_else(|reason: String| json!({"shift_refused": true, "reason": reason}))
        }))
    }
    pub fn exact(&mut self, q: Exact, slot: &Slot) -> Result<ExactPlan, String> {
        let (w, h) = (self.w, self.h);
        let started = epoch();
        session_name(&q.encoder)?;
        let regions = raster::regions(&q.regions, w, h)?;
        // MP-08/MP-10: unprotected patches read only their tiles; protected
        // rasters are masked in full before any tile is cut.
        let masked =
            (!q.patch || !regions.is_empty()).then(|| raster::masked(slot.bytes(), w, h, &regions));
        let pixels: &[u8] = masked.as_deref().unwrap_or(slot.bytes());
        let mut rectangles = Vec::new();
        if q.patch {
            if let Some(rects) = if q.adjacent {
                &slot.adjacent
            } else {
                &slot.tiles
            } {
                for rect in rects {
                    rectangles.extend(clips(&pixels, w, h, *rect, 32, 255, None));
                }
            } else {
                if q.adjacent {
                    return Err("MP-11: adjacent patch unavailable".into());
                }
                if (slot.bounds[2] - slot.bounds[0]) * (slot.bounds[3] - slot.bounds[1]) > 32768 {
                    return Err("MP-11: native patch bound".into());
                }
                rectangles = clips(&pixels, w, h, slot.bounds, 32, 255, None);
            }
        } else {
            let session = self
                .sessions
                .get(&q.encoder)
                .filter(|s| s.regions == regions);
            let rows = session.filter(|s| s.exact).map_or(255, |s| s.dirty);
            rectangles = clips(&pixels, w, h, [0, 0, w, h], 128, rows, session);
        }
        let stride = w as usize * 4;
        let tiles: Vec<([i32; 4], Vec<u8>)> = if q.patch {
            rectangles
                .iter()
                .map(|&[l, t, r, b]| {
                    let mut bytes = Vec::with_capacity(((r - l) * (b - t) * 4) as usize);
                    for row in t..b {
                        let at = row as usize * stride + l as usize * 4;
                        bytes.extend_from_slice(&pixels[at..at + (r - l) as usize * 4]);
                    }
                    ([l, t, r - l, b - t], bytes)
                })
                .collect()
        } else {
            Vec::new()
        };
        // Adjacent patches are cut against the previous readback, others
        // against the base admitted when this slot was read.
        let base = if q.adjacent {
            q.serial.saturating_sub(1)
        } else {
            slot.base
        };
        let pixels = masked.unwrap_or_default();
        let revision = self.sessions.get_mut(&q.encoder).map(|session| {
            session.prepared = Some((
                q.serial,
                if q.patch {
                    Prepared::Patch {
                        base,
                        tiles: tiles.clone(),
                    }
                } else {
                    Prepared::Full(pixels.clone())
                },
            ));
            session.revision
        });
        Ok(ExactPlan {
            pixels,
            tiles,
            rectangles,
            w,
            h,
            patch: q.patch,
            repair_only: q.repair_only,
            revision,
            started,
        })
    }
}
