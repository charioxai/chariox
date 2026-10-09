//! MP-08/MP-10/MP-11: bounded ABI; the C modules never parse control input.
use std::ffi::c_void;
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct RowResult {
    pub row: i32,
    pub y: i32,
    pub height: i32,
    pub key: i32,
    pub sequence: u64,
    pub reference: u64,
    pub bytes: *const u8,
    pub length: usize,
}
impl Default for RowResult {
    fn default() -> Self {
        Self {
            row: 0,
            y: 0,
            height: 0,
            key: 0,
            sequence: 0,
            reference: 0,
            bytes: std::ptr::null(),
            length: 0,
        }
    }
}
extern "C" {
    #[cfg(test)]
    pub(super) fn cx_capture_difference(
        raw: *const u8,
        previous: *const u8,
        width: i32,
        height: i32,
        bounds: *mut i32,
        tiles: *mut i32,
        motion_height: *mut i32,
    ) -> i32;

    pub(super) fn cx_shift_plan(
        raw: *const u8,
        base: *const u8,
        width: i32,
        height: i32,
        out: *mut i32,
    ) -> i32;
    #[cfg(test)]
    pub(super) fn WebPDecodeRGB(
        data: *const u8,
        size: usize,
        width: *mut i32,
        height: *mut i32,
    ) -> *mut u8;

    pub(super) fn cx_capture_open(owner: libc::c_ulong, width: i32, height: i32) -> *mut c_void;
    pub(super) fn cx_capture_close(c: *mut c_void);
    pub(super) fn cx_capture_admit(c: *mut c_void, pixels: *const u8);
    pub(super) fn cx_capture_tiles(c: *mut c_void, out: *mut i32) -> i32;
    pub(super) fn cx_capture_adjacent_tiles(c: *mut c_void, out: *mut i32) -> i32;
    pub(super) fn cx_capture_motion_height(c: *mut c_void) -> i32;
    pub(super) fn cx_capture_shift(c: *mut c_void, out: *mut i32) -> i32;
    pub(super) fn cx_capture_wheel(c: *mut c_void, x: i32, y: i32, dx: i32, dy: i32) -> i32;
    pub(super) fn cx_capture_click(c: *mut c_void, x: i32, y: i32) -> i32;
    pub(super) fn cx_capture_key(c: *mut c_void, keysym: libc::c_ulong, shift: i32) -> i32;
    pub(super) fn cx_capture_plans(c: *mut c_void, enabled: i32);
    pub(super) fn cx_webp_lossless(
        bgrx: *const u8,
        stride: i32,
        width: i32,
        height: i32,
        method: i32,
        quality: i32,
        out: *mut *mut u8,
        length: *mut usize,
    ) -> i32;
    pub(super) fn cx_webp_free(bytes: *mut u8);
    pub(super) fn cx_capture_cpu(c: *mut c_void, out: *mut f64);
    pub(super) fn cx_capture_fd(c: *mut c_void) -> i32;
    pub(super) fn cx_capture_damage(c: *mut c_void) -> i32;
    pub(super) fn cx_capture_read(c: *mut c_void, out: *mut u8, bounds: *mut i32) -> i32;
    pub(super) fn cx_codec_open(
        width: i32,
        height: i32,
        bitrate: i32,
        row_count: i32,
        reduced: i32,
    ) -> *mut c_void;
    pub(super) fn cx_codec_repair_bounds(
        c: *mut c_void,
        source: *const u8,
        overlay: *const u8,
        video_rows: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        bounds: *mut i32,
    );
    pub(super) fn cx_codec_diagnostic(c: *mut c_void) -> *const std::ffi::c_char;
    pub(super) fn cx_codec_backend(c: *mut c_void) -> i32;
    pub(super) fn cx_codec_reduced(c: *mut c_void) -> i32;
    pub(super) fn cx_codec_rate(c: *mut c_void, bitrate: i32) -> i32;
    pub(super) fn cx_codec_cpu(c: *mut c_void, out: *mut f64);
    pub(super) fn cx_codec_close(c: *mut c_void);
    pub(super) fn cx_codec_encode(
        c: *mut c_void,
        source: *const u8,
        resets: u32,
        regions: *const Rect,
        count: usize,
        results: *mut RowResult,
    ) -> i32;
    pub(super) fn cx_mask(
        pixels: *mut u8,
        width: i32,
        height: i32,
        regions: *const Rect,
        count: usize,
    );
}
pub(super) struct Capture(pub *mut c_void);
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe { cx_capture_close(self.0) }
    }
}
pub(super) struct Codec(pub *mut c_void);
// MP-08/MP-10: a codec is used by one thread at a time under the sessions lock.
unsafe impl Send for Codec {}
impl Drop for Codec {
    fn drop(&mut self) {
        unsafe { cx_codec_close(self.0) }
    }
}
