//! Keep artifact downloads within their byte cap and preserve recovery space.
use std::io::{self, Read, Write};

const RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(super) fn copy(
    mut source: impl Read,
    mut destination: impl Write,
    max_bytes: u64,
    mut available: impl FnMut() -> io::Result<u64>,
) -> io::Result<()> {
    let admit = |free: u64, next: u64| {
        if free.saturating_sub(RESERVE_BYTES) < next || free < RESERVE_BYTES {
            Err(io::Error::other("insufficient artifact download disk headroom"))
        } else { Ok(()) }
    };
    admit(available()?, max_bytes)?;
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 { return Ok(()); }
        if count as u64 > max_bytes.saturating_sub(copied) {
            return Err(io::Error::other("artifact exceeds download byte limit"));
        }
        admit(available()?, count as u64)?;
        destination.write_all(&buffer[..count])?;
        copied += count as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_bytes_are_written_without_download_headroom() {
        let mut output = Vec::new();
        let error = copy(&b"archive"[..], &mut output, 100, || Ok(RESERVE_BYTES + 99)).unwrap_err();
        assert!(error.to_string().contains("headroom"));
        assert!(output.is_empty());
    }

    #[test]
    fn concurrent_disk_use_stops_before_consuming_the_reserve() {
        let input = vec![7; 131072];
        let mut output = Vec::new();
        let mut free = [RESERVE_BYTES + 131072, RESERVE_BYTES + 65536, RESERVE_BYTES].into_iter();
        let error = copy(&input[..], &mut output, input.len() as u64, || Ok(free.next().unwrap())).unwrap_err();
        assert!(error.to_string().contains("headroom"));
        assert_eq!(output.len(), 65536);
    }

    #[test]
    fn accepted_bytes_are_exact_and_oversized_bodies_are_rejected() {
        let mut output = Vec::new();
        copy(&b"archive"[..], &mut output, 7, || Ok(u64::MAX)).unwrap();
        assert_eq!(output, b"archive");
        output.clear();
        assert!(copy(&b"archive!"[..], &mut output, 7, || Ok(u64::MAX)).is_err());
        assert!(output.is_empty());
    }
}
