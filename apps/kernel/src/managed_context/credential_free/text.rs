//! MP-08/MP-11: classify bytes as text, an allow-listed asset or unclassifiable.

// MP-11: high-signal formats are checked in every text role, even with no
// assignment/header name. This finite set is a guard, not secret recognition.
pub(super) fn credential_format(text: &str) -> bool {
    if text.contains("PRIVATE KEY-----") {
        return true;
    }
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '-' | '.'))
        .any(|word| {
            [
                ("ghp_", 36),
                ("gho_", 36),
                ("ghu_", 36),
                ("ghs_", 36),
                ("ghr_", 36),
                ("github_pat_", 22),
                ("sk-", 20),
                ("xoxb-", 10),
                ("xoxp-", 10),
                ("xoxa-", 10),
                ("xoxr-", 10),
                ("xoxs-", 10),
            ]
            .iter()
            .any(|(prefix, minimum)| {
                word.strip_prefix(prefix)
                    .is_some_and(|value| value.len() >= *minimum)
            }) || ["AKIA", "ASIA"].iter().any(|prefix| {
                word.strip_prefix(prefix).is_some_and(|value| {
                    value.len() == 16
                        && value
                            .bytes()
                            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
                })
            }) || (word.starts_with("eyJ") && {
                let parts = word.split('.').collect::<Vec<_>>();
                parts.len() == 3 && parts.iter().all(|part| part.len() >= 8)
            })
        })
}

/// MP-08/MP-11: the one byte classification every Copy boundary uses.
pub(super) enum Content<'a> {
    /// Strict UTF-8 (optional BOM) with no binary control; the text policy applies.
    Text(&'a str),
    /// An allow-listed asset: its extension and signature agree and it is not text.
    Asset,
    /// Anything else: Vault-routed at the source and refused by the target.
    Unclassified,
}

// MP-08/MP-11: opaque assets are explicit (extension, signature) pairs, never
// "whatever failed to decode". Offsets are where the signature starts.
const ASSETS: &[(&[&str], usize, &[u8])] = &[
    (&["png"], 0, b"\x89PNG\r\n\x1a\n"),
    (&["jpg", "jpeg"], 0, b"\xff\xd8\xff"),
    (&["gif"], 0, b"GIF87a"),
    (&["gif"], 0, b"GIF89a"),
    (&["webp"], 8, b"WEBP"),
    (&["ico"], 0, b"\0\0\x01\0"),
    (&["avif", "mp4", "mov", "m4a"], 4, b"ftyp"),
    (&["woff"], 0, b"wOFF"),
    (&["woff2"], 0, b"wOF2"),
    (&["ttf"], 0, b"\0\x01\0\0"),
    (&["otf"], 0, b"OTTO"),
    (&["pdf"], 0, b"%PDF-"),
    (&["zip", "jar"], 0, b"PK\x03\x04"),
    (&["gz", "tgz"], 0, b"\x1f\x8b"),
    (&["wasm"], 0, b"\0asm"),
    (&["webm"], 0, b"\x1a\x45\xdf\xa3"),
    (&["ogg"], 0, b"OggS"),
    (&["wav"], 8, b"WAVE"),
    (&["mp3"], 0, b"ID3"),
];

pub(super) fn classify<'a>(path: &str, bytes: &'a [u8]) -> Content<'a> {
    // A UTF-16/32 BOM does not prove the encoding of the bytes after it: a
    // byte-order or encoding mismatch decodes to valid, unrelated characters.
    let wide = [&[0xff, 0xfe][..], &[0xfe, 0xff], &[0, 0, 0xfe, 0xff]];
    if wide.iter().any(|bom| bytes.starts_with(bom)) {
        return Content::Unclassified;
    }
    let text = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(text) {
        // NULs, BOM-less wide text and binary controls cannot be classified.
        if !text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t' | '\x0c'))
        {
            return Content::Text(text);
        }
    }
    let extension = path.rsplit_once('.').map_or("", |(_, extension)| extension);
    if !bytes.starts_with(&[0xef, 0xbb, 0xbf])
        && ASSETS.iter().any(|(extensions, offset, signature)| {
            extensions.contains(&extension)
                && bytes
                    .get(*offset..)
                    .is_some_and(|rest| rest.starts_with(signature))
        })
    {
        return Content::Asset;
    }
    Content::Unclassified
}
