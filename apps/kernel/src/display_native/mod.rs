//! MP-08/MP-10/MP-11: kernel-owned Linux capture/codec worker. Node supplies
//! admitted control and trusted CDP fences; no pixels traverse its motion loop.
mod exact;
mod ffi;
mod raster;
mod sessions;
mod worker;
pub(crate) use crate::runtime::browser_controller_process::RASTER_SLOT_NAMES;
pub fn codec_decoder_library() -> &'static str {
    unsafe { std::ffi::CStr::from_ptr(ffi::cx_codec_decoder_library()) }
        .to_str()
        .expect("static decoder library")
}
pub fn probe_codec() -> Result<(), String> {
    if unsafe { ffi::cx_codec_available() } == 0 {
        return Err("MP-08/MP-10: native codec unavailable".into());
    }
    Ok(())
}
pub fn run() -> Result<(), String> {
    worker::run()
}

#[cfg(test)]
mod tests;
