//! MP-08 / MP-09 / MP-10 / MP-11: debug-build controls for the explicit EA2 drills.
use super::*;

pub(super) fn limit() -> Result<usize, DaemonError> {
    #[cfg(debug_assertions)]
    if let Some(value) = std::env::var_os("CHARIOX_TEST_OUTSIDE_PENDING_LIMIT") {
        let limit = value
            .to_str()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|limit| (4..=PENDING_LIMIT).contains(limit))
            .ok_or_else(|| {
                error("outside test pending limit must be an integer from 4 to 100000")
            })?;
        return Ok(limit);
    }
    Ok(PENDING_LIMIT)
}
pub(super) fn now() -> Result<u64, DaemonError> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("CHARIOX_TEST_OUTSIDE_CLOCK_MS_FILE") {
        use std::io::Read;
        let mut value = String::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(64).read_to_string(&mut value))
            .map_err(|_| error("outside test clock could not be read"))?;
        return value
            .trim()
            .parse()
            .map_err(|_| error("outside test clock must contain epoch milliseconds"));
    }
    Ok(crate::session::unix_epoch_ms())
}
