//! MP-08/MP-11: decode supported text before inspection; unknown bytes fail closed.
use super::refused;
use crate::error::DaemonError;
use std::borrow::Cow;

pub(super) fn decode(bytes: &[u8]) -> Result<Cow<'_, str>, DaemonError> {
    // UTF-32LE shares the UTF-16LE prefix, so check four-byte BOMs first.
    let text = if bytes.starts_with(&[0xff, 0xfe, 0, 0]) || bytes.starts_with(&[0, 0, 0xfe, 0xff]) {
        let little = bytes[0] == 0xff;
        let bytes = &bytes[4..];
        if bytes.len() % 4 != 0 {
            return Err(refused());
        }
        let text = bytes
            .chunks_exact(4)
            .map(|chunk| {
                let unit = [chunk[0], chunk[1], chunk[2], chunk[3]];
                char::from_u32(if little {
                    u32::from_le_bytes(unit)
                } else {
                    u32::from_be_bytes(unit)
                })
                .ok_or_else(refused)
            })
            .collect::<Result<String, _>>()?;
        Cow::Owned(text)
    } else if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let little = bytes[0] == 0xff;
        let bytes = &bytes[2..];
        if bytes.len() % 2 != 0 {
            return Err(refused());
        }
        let units = bytes.chunks_exact(2).map(|chunk| {
            if little {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        });
        Cow::Owned(
            char::decode_utf16(units)
                .map(|ch| ch.map_err(|_| refused()))
                .collect::<Result<String, _>>()?,
        )
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        Cow::Borrowed(std::str::from_utf8(bytes).map_err(|_| refused())?)
    };
    // NULs/BOM-less wide text and binary controls cannot be classified safely.
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(refused());
    }
    Ok(text)
}
