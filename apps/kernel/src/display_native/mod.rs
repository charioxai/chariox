//! MP-08/MP-10/MP-11: kernel-owned Linux capture/codec worker. Node supplies
//! admitted control and trusted CDP fences; no pixels traverse its motion loop.
mod exact;
mod ffi;
mod raster;
mod sessions;
mod worker;
/// MP-08/MP-10/MP-11: shared capture slot files (`<pool>/<index>`); the
/// supervisor's crash cleanup reclaims exactly these.
pub(crate) const RASTER_SLOT_NAMES: [&str; 6] = ["0", "1", "2", "3", "4", "5"];
pub fn run() -> Result<(), String> {
    worker::run()
}

#[cfg(test)]
mod tests;
