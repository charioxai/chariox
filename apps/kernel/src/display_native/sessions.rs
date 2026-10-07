//! MP-08/MP-10/MP-11: native codec sessions, delivered references and exact repairs.
use super::{
    ffi::{self, Codec, Rect, RowResult},
    raster::{self, Region, Slot},
    worker::epoch,
};
use base64::{engine::general_purpose::STANDARD, Engine};
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
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Exact {
    pub id: u64,
    pub encoder: String,
    pub serial: u64,
    pub regions: Vec<Region>,
    pub limit: usize,
    pub patch: bool,
}
struct Session {
    codec: Codec,
    bitrate: i32,
    stripes: bool,
    regions: Vec<Rect>,
    dirty: u8,
    exact: bool,
    revision: u64,
    delivered: u64,
    video_rows: u8,
    overlay: Option<Vec<u8>>,
    prepared: Option<(u64, Vec<u8>)>,
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
fn packet(root: &PathBuf, rows: &[Value]) -> Result<Value, String> {
    let bytes = serde_json::to_vec(rows).map_err(|_| "MP-11: native packet JSON")?;
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err("MP-11: native packet bound".into());
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
fn tiles(
    pixels: &[u8],
    w: i32,
    h: i32,
    bounds: [i32; 4],
    block: i32,
    rows: u8,
    session: Option<&Session>,
) -> Result<Vec<Value>, String> {
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
            if let Some(s) = session.filter(|s| s.delivered == s.revision) {
                unsafe {
                    ffi::cx_codec_repair_bounds(
                        s.codec.0,
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
            let [x, y, right, bottom] = clip;
            let (width, height) = (right - x, bottom - y);
            let offset = ((y * w + x) * 4) as usize;
            let bytes = raster::png(
                &pixels[offset..],
                width as u32,
                height as u32,
                (w * 4) as usize,
            )?;
            output.push(json!({"x":x,"y":y,"width":width,"height":height,"data_base64":STANDARD.encode(bytes)}));
        }
    }
    Ok(output)
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
    pub fn damage(&mut self, bounds: [i32; 4]) {
        for s in self.sessions.values_mut() {
            for r in 0..8 {
                if bounds[1] < 2 * ((self.h / 2 * (r + 1)) / 8)
                    && bounds[3] > 2 * ((self.h / 2 * r) / 8)
                {
                    s.dirty |= 1 << r;
                }
            }
        }
    }
    pub fn commit(&mut self, name: &str, serial: u64) {
        if let Some(s) = self.sessions.get_mut(name) {
            if s.prepared.as_ref().is_some_and(|(id, _)| *id == serial) {
                let (_, pixels) = s.prepared.take().unwrap();
                s.overlay = Some(pixels);
                s.video_rows = 0;
                s.dirty = 0;
                s.exact = true;
            }
        }
    }
    pub fn delivered(&mut self, name: &str, revision: u64) {
        if let Some(s) = self.sessions.get_mut(name) {
            if revision <= s.revision {
                s.delivered = s.delivered.max(revision);
            }
        }
    }
    pub fn encode(&mut self, q: Encode, slot: &Slot) -> Result<Value, String> {
        let (w, h) = (self.w, self.h);
        let sessions = &mut self.sessions;
        let codec_revision = &mut self.revision;
        let root = &self.root;

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
        if sessions.get(&q.encoder).is_some_and(|s| {
            s.bitrate != q.bitrate || s.regions != regions || s.stripes != q.stripes
        }) {
            sessions.remove(&q.encoder);
        }
        if !sessions.contains_key(&q.encoder) {
            if sessions.len() >= 8 {
                return Err("MP-11: native encoder count".into());
            }
            let codec = Codec(unsafe {
                ffi::cx_codec_open(w, h, q.bitrate, if q.stripes { 8 } else { 1 })
            });
            if codec.0.is_null() {
                return Err("MP-10: native codec unavailable".into());
            }
            sessions.insert(
                q.encoder.clone(),
                Session {
                    codec,
                    bitrate: q.bitrate,
                    stripes: q.stripes,
                    regions,
                    dirty: 255,
                    exact: false,
                    revision: 0,
                    delivered: 0,
                    video_rows: 0,
                    overlay: None,
                    prepared: None,
                },
            );
            resets = 255;
        }
        let session = sessions.get_mut(&q.encoder).unwrap();
        let mut results = [RowResult::default(); 8];
        let count = unsafe {
            ffi::cx_codec_encode(
                session.codec.0,
                slot.pixels,
                resets,
                session.regions.as_ptr(),
                session.regions.len(),
                results.as_mut_ptr(),
            )
        };
        if count == -2 {
            session.dirty = 255;
            session.exact = false;
            return Ok(json!({"dropped":true,"backend":"native-x264","converter":"libyuv"}));
        }
        if !(0..=8).contains(&count) {
            return Err("MP-11: native encode unavailable".into());
        }
        let mut rows = Vec::new();
        let mut headers = Vec::new();
        for r in &results[..count as usize] {
            if r.bytes.is_null() || r.length == 0 || r.length > 1024 * 1024 {
                return Err("MP-11: native row bound".into());
            }
            let data = unsafe { std::slice::from_raw_parts(r.bytes, r.length) };
            let header = json!({"row":r.row,"y":r.y,"height":r.height,"codec":"avc1.420033","key":r.key!=0,"sequence":r.sequence,"reference_sequence":if r.key!=0 {None}else{Some(r.reference)}});
            let mut row = header.clone();
            row["data_base64"] = STANDARD.encode(data).into();
            rows.push(row);
            headers.push(header);
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
        let mut cpu = [0f64; 6];
        unsafe {
            ffi::cx_codec_cpu(session.codec.0, cpu.as_mut_ptr());
        }
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
            Some(packet(&root, &rows)?)
        };
        spans.push(json!(["codec_packetize", encoded_at, epoch()]));
        Ok(
            json!({"stripes":headers,"packet":descriptor,"backend":if unsafe {ffi::cx_codec_backend(session.codec.0)}==1 {"native-vaapi"}else{"native-x264"},"hardware_fallback":unsafe {ffi::cx_codec_backend(session.codec.0)}==2,"converter":"libyuv","workers":1,"timings":spans,"whole":!q.stripes,"revision":session.revision}),
        )
    }
    pub fn exact(&mut self, q: Exact, slot: &Slot) -> Result<Value, String> {
        let (w, h) = (self.w, self.h);
        let sessions = &mut self.sessions;

        let started = epoch();
        session_name(&q.encoder)?;
        if q.limit < 24000 || q.limit > 192000 {
            return Err("MP-11: native repair budget".into());
        }
        let regions = raster::regions(&q.regions, w, h)?;
        let pixels = raster::masked(slot.bytes(), w, h, &regions);
        let mut value = json!({"width":w,"height":h,"native_exact":true});
        if q.patch {
            if (slot.bounds[2] - slot.bounds[0]) * (slot.bounds[3] - slot.bounds[1]) > 32768 {
                return Err("MP-11: native patch bound".into());
            }
            value["native_tiles"] = tiles(&pixels, w, h, slot.bounds, 32, 255, None)?.into();
        } else {
            let png = raster::png(&pixels, w as u32, h as u32, (w * 4) as usize)?;
            let full = STANDARD.encode(png);
            value["data_base64"] = full.into();
            let session = sessions.get(&q.encoder);
            let rows = session
                .filter(|s| s.exact && s.regions == regions)
                .map_or(255, |s| s.dirty);
            value["repair_tiles"] = tiles(
                &pixels,
                w,
                h,
                [0, 0, w, h],
                128,
                rows,
                session.filter(|s| s.regions == regions),
            )?
            .into();
        }
        if let Some(session) = sessions.get_mut(&q.encoder) {
            value["native_revision"] = session.revision.into();
            session.prepared = Some((q.serial, pixels));
        }
        value["timings"] = json!([["native_exact_prepare", started, epoch()]]);
        Ok(value)
    }
}
