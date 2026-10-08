//! MP-08/MP-10/MP-11: immutable slot leases, intersected masks and exact PNG.
use super::ffi::{self, Rect};
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
// MP-11: a codec job owns the mapping while it reads it, including after a
// control timeout releases the client lease or the supervisor exits.
struct Mapping {
    address: usize,
    length: usize,
}
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.address as *mut libc::c_void, self.length);
        }
    }
}
pub(super) struct CodecLease(Arc<Mapping>);
impl CodecLease {
    pub fn pixels(&self) -> *const u8 {
        self.0.address as *const u8
    }
}
pub(super) struct Slot {
    pub pixels: *mut u8,
    pub length: usize,
    pub serial: Option<u64>,
    pub bounds: [i32; 4],
    pub tiles: Option<Vec<[i32; 4]>>,
    pub adjacent: Option<Vec<[i32; 4]>>,
    /// MP-08/MP-10: proved vertical scroll plan against the previous readback.
    pub shift: Option<Shift>,
    /// The admitted serial that `tiles` were compared against at readback.
    pub base: u64,
    file: File,
    path: PathBuf,
    mapping: Arc<Mapping>,
}
impl Slot {
    pub fn create(root: &Path, index: usize, length: usize) -> Result<Self, String> {
        let path = root.join(index.to_string());
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|_| "MP-11: native pool open")?;
        file.set_len(length as u64)
            .map_err(|_| "MP-11: native pool size")?;
        let pixels = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if pixels == libc::MAP_FAILED {
            let _ = std::fs::remove_file(&path);
            return Err("MP-11: native pool map".into());
        }
        Ok(Self {
            pixels: pixels.cast(),
            length,
            serial: None,
            bounds: [0; 4],
            tiles: None,
            adjacent: None,
            shift: None,
            base: 0,
            file,
            path,
            mapping: Arc::new(Mapping {
                address: pixels as usize,
                length,
            }),
        })
    }
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.pixels, self.length) }
    }
    pub fn codec_lease(&self) -> CodecLease {
        CodecLease(self.mapping.clone())
    }
    pub fn available(&self) -> bool {
        self.serial.is_none() && Arc::strong_count(&self.mapping) == 1
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        // Remove only this exact private regular inode; the kernel additionally
        // owns reclamation after abrupt supervisor/worker death.
        if std::fs::symlink_metadata(&self.path)
            .ok()
            .zip(self.file.metadata().ok())
            .is_some_and(|(a, b)| a.is_file() && a.dev() == b.dev() && a.ino() == b.ino())
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
/// MP-08/MP-10: destination rectangles [x,y,w,h]; moves copy from y-dy.
#[derive(Clone, serde::Serialize)]
pub(super) struct Shift {
    pub dy: i32,
    pub dirty_pixels: i32,
    pub moves: Vec<[i32; 4]>,
    pub dirty: Vec<[i32; 4]>,
}
impl Shift {
    pub fn read(capture: *mut std::ffi::c_void) -> Option<Self> {
        let mut out = [0i32; 4 + 128 * 4];
        if unsafe { ffi::cx_capture_shift(capture, out.as_mut_ptr()) } != 1 {
            return None;
        }
        Self::decode(&out)
    }
    /// The same proved decomposition against a retained exact raster.
    pub fn plan(raw: &[u8], base: &[u8], w: i32, h: i32) -> Option<Self> {
        let mut out = [0i32; 4 + 128 * 4];
        if raw.len() != (w * h * 4) as usize
            || base.len() != raw.len()
            || unsafe { ffi::cx_shift_plan(raw.as_ptr(), base.as_ptr(), w, h, out.as_mut_ptr()) }
                != 1
        {
            return None;
        }
        Self::decode(&out)
    }
    fn decode(out: &[i32; 4 + 128 * 4]) -> Option<Self> {
        let (moves, dirty) = (out[1] as usize, out[2] as usize);
        if moves > 64 || dirty > 64 {
            return None;
        }
        let rects = |start: usize, count: usize| {
            out[start..start + count * 4]
                .chunks_exact(4)
                .map(|r| [r[0], r[1], r[2], r[3]])
                .collect::<Vec<_>>()
        };
        Some(Self {
            dy: out[0],
            dirty_pixels: out[3],
            moves: rects(4, moves),
            dirty: rects(4 + moves * 4, dirty),
        })
    }
}
/// MP-08/MP-10: one lossless WebP image of a BGRX raster region.
pub(super) fn webp(
    pixels: &[u8],
    stride: usize,
    rect: [i32; 4],
    method: i32,
    quality: i32,
) -> Result<Vec<u8>, String> {
    let [x, y, w, h] = rect;
    let start = y as usize * stride + x as usize * 4;
    if x < 0
        || y < 0
        || w < 1
        || h < 1
        || (x + w) as usize * 4 > stride
        || start + (h as usize - 1) * stride + w as usize * 4 > pixels.len()
    {
        return Err("MP-11: lossless region bounds".into());
    }
    let (mut out, mut length) = (std::ptr::null_mut(), 0usize);
    if unsafe {
        ffi::cx_webp_lossless(
            pixels[start..].as_ptr(),
            stride as i32,
            w,
            h,
            method,
            quality,
            &mut out,
            &mut length,
        )
    } != 0
    {
        return Err("MP-10: lossless encode".into());
    }
    let bytes = unsafe { std::slice::from_raw_parts(out, length) }.to_vec();
    unsafe { ffi::cx_webp_free(out) };
    Ok(bytes)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Region {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
pub(super) fn regions(input: &[Region], w: i32, h: i32) -> Result<Vec<Rect>, String> {
    if input.len() > 50000 {
        return Err("MP-11: native mask count".into());
    }
    input.iter().try_fold(Vec::new(), |mut rects, r| {
        if ![r.x, r.y, r.width, r.height, r.x + r.width, r.y + r.height]
            .iter()
            .all(|v| v.is_finite())
            || r.width < 0.
            || r.height < 0.
        {
            return Err("MP-11: native mask geometry".into());
        }
        let rect = Rect {
            left: r.x.floor().clamp(0., w as f64) as i32,
            top: r.y.floor().clamp(0., h as f64) as i32,
            right: (r.x + r.width).ceil().clamp(0., w as f64) as i32,
            bottom: (r.y + r.height).ceil().clamp(0., h as f64) as i32,
        };
        if rect.left < rect.right && rect.top < rect.bottom {
            rects.push(rect);
        }
        Ok(rects)
    })
}
pub(super) fn masked(raw: &[u8], w: i32, h: i32, regions: &[Rect]) -> Vec<u8> {
    let mut pixels = raw.to_vec();
    unsafe {
        ffi::cx_mask(pixels.as_mut_ptr(), w, h, regions.as_ptr(), regions.len());
    }
    pixels
}
pub(super) fn png(pixels: &[u8], w: u32, h: u32, stride: usize) -> Result<Vec<u8>, String> {
    // Exact indexed PNG often makes scrolling text fit one negotiated repair
    // batch. The palette is proved per RGB value; no quantization is allowed.
    let mut palette = Vec::new();
    // MP-08/MP-10: a bounded exact RGB dictionary avoids a keyed hash per
    // viewport pixel. Collision probing checks the full RGB; no quantization.
    let mut colors = [0u32; 512];
    let mut indices = [0u8; 512];
    let mut count = 0;
    let mut last = (0u32, 0u8);
    let mut indexed = Vec::with_capacity(w as usize * h as usize);
    let mut fits = true;
    'rows: for row in pixels.chunks(stride).take(h as usize) {
        for p in row[..w as usize * 4].chunks_exact(4) {
            let color = ((p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32) + 1;
            let index = if color == last.0 {
                last.1
            } else {
                let mut entry = (color.wrapping_mul(0x9e3779b1) >> 23) as usize;
                while colors[entry] != 0 && colors[entry] != color {
                    entry = (entry + 1) & 511;
                }
                if colors[entry] == 0 {
                    if count == 256 {
                        fits = false;
                        break 'rows;
                    }
                    colors[entry] = color;
                    indices[entry] = count as u8;
                    count += 1;
                    palette.extend_from_slice(&[p[2], p[1], p[0]]);
                }
                let index = indices[entry];
                last = (color, index);
                index
            };
            indexed.push(index);
        }
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, w, h);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Default);
        encoder.set_filter(png::FilterType::NoFilter);
        if fits {
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_palette(palette);
        } else {
            encoder.set_color(png::ColorType::Rgb);
            indexed.clear();
            for row in pixels.chunks(stride).take(h as usize) {
                for p in row[..w as usize * 4].chunks_exact(4) {
                    indexed.extend_from_slice(&[p[2], p[1], p[0]]);
                }
            }
        }
        encoder
            .write_header()
            .map_err(|_| "MP-10: exact PNG header")?
            .write_image_data(&indexed)
            .map_err(|_| "MP-10: exact PNG raster")?;
    }
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("MP-10: exact PNG bound".into());
    }
    Ok(bytes)
}
