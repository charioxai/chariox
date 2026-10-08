//! MP-08/MP-10/MP-11: kernel-owned Linux capture/codec worker. Node supplies
//! admitted control and trusted CDP fences; no pixels traverse its motion loop.
mod exact;
mod ffi;
mod raster;
mod sessions;
mod worker;
pub fn run() -> Result<(), String> {
    worker::run()
}

#[cfg(test)]
mod tests;
