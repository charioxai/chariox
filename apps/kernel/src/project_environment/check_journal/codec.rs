//! MP-08 / MP-10 / MP-11: bounded private snapshot compression, outside the wire contract.
use flate2::{Compression, Decompress, FlushDecompress, Status};
use std::borrow::Cow;
use std::io::{self, Write};
const TAG: &[u8] = b"\0CXJZ1";

pub(super) fn encode(bytes: &[u8]) -> io::Result<Cow<'_, [u8]>> {
    // Check snapshots are JSON objects. Keep small snapshots and non-JSON
    // journal fixtures raw; the physical record and checksum stay unchanged.
    if bytes.len() < 4096 || bytes.first() != Some(&b'{') {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut writer = flate2::write::ZlibEncoder::new(Vec::new(), Compression::fast());
    writer.write_all(bytes)?;
    let compressed = writer.finish()?;
    if compressed.len() + TAG.len() >= bytes.len() {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut stored = Vec::with_capacity(TAG.len() + compressed.len());
    stored.extend_from_slice(TAG);
    stored.extend_from_slice(&compressed);
    Ok(Cow::Owned(stored))
}

pub(super) fn decode(bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    let Some(input) = bytes.strip_prefix(TAG) else {
        return Ok(bytes);
    };
    let invalid = || io::Error::from(io::ErrorKind::InvalidData);
    let mut decoder = Decompress::new(true);
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(
                &input[before_in as usize..],
                &mut buffer,
                FlushDecompress::None,
            )
            .map_err(|_| invalid())?;
        let written = (decoder.total_out() - before_out) as usize;
        if output.len() + written > super::LIMIT as usize {
            return Err(invalid());
        }
        output.extend_from_slice(&buffer[..written]);
        if status == Status::StreamEnd {
            return if decoder.total_in() == input.len() as u64 {
                Ok(output)
            } else {
                Err(invalid())
            };
        }
        if decoder.total_in() == before_in && written == 0 {
            return Err(invalid());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(n: usize) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"public_status": "ready ".repeat(n)})).unwrap()
    }
    #[test]
    fn compressed_and_existing_raw_snapshots_round_trip_exactly() {
        for raw in [b"{}".to_vec(), snapshot(10_000)] {
            assert_eq!(decode(raw.clone()).unwrap(), raw);
            let stored = encode(&raw).unwrap();
            if raw.len() >= 4096 {
                assert!(stored.len() < 4096);
            }
            assert_eq!(decode(stored.into_owned()).unwrap(), raw);
        }
    }
    #[test]
    fn compressed_expansion_incomplete_streams_and_trailing_bytes_fail_closed() {
        let raw = snapshot(1000);
        let stored = encode(&raw).unwrap().into_owned();
        assert!(stored.starts_with(TAG));
        assert!(decode(stored[..stored.len() - 2].to_vec()).is_err());
        let mut trailing = stored;
        trailing.extend_from_slice(b"trailing");
        assert!(decode(trailing).is_err());
        let mut writer = flate2::write::ZlibEncoder::new(TAG.to_vec(), Compression::fast());
        writer
            .write_all(&vec![b'x'; super::super::LIMIT as usize + 1])
            .unwrap();
        assert!(decode(writer.finish().unwrap()).is_err());
    }
}
