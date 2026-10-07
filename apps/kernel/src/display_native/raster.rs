//! MP-08/MP-10/MP-11: immutable slot leases, intersected masks and exact PNG.
use super::ffi::{self, Rect};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
pub(super) struct Slot {
    pub pixels: *mut u8,
    pub length: usize,
    pub serial: Option<u64>,
    pub bounds: [i32; 4],
    file: File,
    path: PathBuf,
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
            file,
            path,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.pixels, self.length) }
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.pixels.cast(), self.length);
        }
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
    let mut colors = HashMap::new();
    let mut indexed = Vec::with_capacity(w as usize * h as usize);
    let mut fits = true;
    'rows: for row in pixels.chunks(stride).take(h as usize) {
        for p in row[..w as usize * 4].chunks_exact(4) {
            let color = [p[2], p[1], p[0]];
            let index = if let Some(index) = colors.get(&color) {
                *index
            } else {
                if colors.len() == 256 {
                    fits = false;
                    break 'rows;
                }
                let index = colors.len() as u8;
                colors.insert(color, index);
                palette.extend_from_slice(&color);
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
