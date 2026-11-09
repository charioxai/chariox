//! MP-08/MP-10/MP-11: Cloud enrollment adopts the OS owner's gate work in place.
use super::*;

pub(super) fn enroll_owner_in(
    _tx: &Transaction<'_>,
    _session: &str,
    _old: &str,
    _new: &str,
) -> Result<()> {
    Ok(())
}
