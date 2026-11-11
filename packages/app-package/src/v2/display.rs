//! The optional signed presentation of an App: `displayName` and `icon`.
//!
//! The icon is shown by every client, so it is checked when a package is
//! packed and again when it is verified: a small PNG, or an SVG that carries
//! no script and refers to nothing outside itself.
use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine};

use crate::{archive::validate_path, ErrorCode, Limits, PackageError, Result};

pub const MAX_DISPLAY_NAME_CHARS: usize = 64;
/// Icons travel inside every installation list, so they stay small.
pub const MAX_ICON_BYTES: usize = 32 * 1024;
pub const MAX_ICON_PIXELS: u32 = 512;

const PNG_IEND_CHUNK: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82];
/// A 512 by 512 image with four 16-bit samples per pixel.
const MAX_DECODED_ICON_BYTES: usize = MAX_ICON_PIXELS as usize * MAX_ICON_PIXELS as usize * 8;

fn invalid(message: &'static str) -> PackageError {
    PackageError::new(ErrorCode::InvalidManifest, message)
}

pub(super) fn validate_display_name(name: &str) -> Result<()> {
    let length = name.chars().count();
    if length == 0
        || length > MAX_DISPLAY_NAME_CHARS
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(invalid(
            "displayName must be 1 to 64 characters, trimmed, without control characters",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IconKind {
    Png,
    Svg,
}

impl IconKind {
    fn of(path: &str) -> Option<Self> {
        match path.rsplit_once('.')?.1 {
            "png" => Some(Self::Png),
            "svg" => Some(Self::Svg),
            _ => None,
        }
    }

    fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Svg => "image/svg+xml",
        }
    }
}

pub(super) fn validate_icon_path(path: &str, limits: &Limits) -> Result<()> {
    validate_path(path, limits)?;
    if IconKind::of(path).is_none() {
        return Err(invalid("icon must be a .png or .svg file in the package"));
    }
    Ok(())
}

/// Checks the icon file the manifest names. The caller has already made sure
/// the path is listed in the payload.
pub(super) fn validate_icon(path: &str, files: &BTreeMap<String, &[u8]>) -> Result<()> {
    let bytes = files
        .get(path)
        .ok_or_else(|| invalid("icon is not part of the package"))?;
    let kind = IconKind::of(path).ok_or_else(|| invalid("icon must be a .png or .svg file"))?;
    if bytes.len() > MAX_ICON_BYTES {
        return Err(invalid("icon exceeds 32 KiB"));
    }
    match kind {
        IconKind::Png => check_png(bytes),
        IconKind::Svg => check_svg(bytes),
    }
}

/// `data:` URL of an icon that `validate_icon` accepted, `None` for any
/// other path.
pub fn icon_data_url(path: &str, bytes: &[u8]) -> Option<String> {
    let kind = IconKind::of(path)?;
    Some(format!(
        "data:{};base64,{}",
        kind.media_type(),
        STANDARD.encode(bytes)
    ))
}

/// Decodes the whole image, because a client cannot show a PNG that does not
/// decode. The header is read first and its size checked against the pixel cap
/// before any buffer is sized from it, so the decode cannot allocate more than
/// the largest permitted icon needs.
fn check_png(bytes: &[u8]) -> Result<()> {
    let bad = || invalid("icon is not a decodable PNG up to 512 by 512 pixels");
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODED_ICON_BYTES,
    });
    let mut reader = decoder.read_info().map_err(|_| bad())?;
    let info = reader.info();
    if !(1..=MAX_ICON_PIXELS).contains(&info.width)
        || !(1..=MAX_ICON_PIXELS).contains(&info.height)
        || info.is_animated()
    {
        return Err(bad());
    }
    let size = reader.output_buffer_size();
    if size > MAX_DECODED_ICON_BYTES {
        return Err(bad());
    }
    let mut image = vec![0; size];
    reader.next_frame(&mut image).map_err(|_| bad())?;
    // Reads the remaining chunks up to IEND, checking each CRC. The decoder
    // stops there, so the file must also end there.
    reader.finish().map_err(|_| bad())?;
    if !bytes.ends_with(&PNG_IEND_CHUNK) {
        return Err(bad());
    }
    Ok(())
}

const FORBIDDEN_ELEMENTS: [&str; 8] = [
    "script",
    "foreignobject",
    "iframe",
    "embed",
    "object",
    "audio",
    "video",
    "canvas",
];

/// A lexical check, deliberately stricter than the SVG grammar: anything that
/// can run code or fetch something is refused rather than interpreted.
fn check_svg(bytes: &[u8]) -> Result<()> {
    let bad = |message| invalid(message);
    let text = std::str::from_utf8(bytes).map_err(|_| bad("icon SVG must be UTF-8"))?;
    let lower = text.to_ascii_lowercase();
    if lower.contains('\0')
        || lower.contains("<!doctype")
        || lower.contains("<!entity")
        || lower.contains("<?xml-stylesheet")
        || lower.contains("@import")
        || lower.contains("&#")
        || lower.contains("javascript:")
    {
        return Err(bad(
            "icon SVG carries a doctype, entity, import or script reference",
        ));
    }
    let mut root_seen = false;
    let mut cursor = 0;
    while let Some(offset) = lower[cursor..].find('<') {
        let start = cursor + offset;
        let rest = &lower[start + 1..];
        if rest.starts_with("!--") {
            cursor = start + rest.find("-->").map_or(rest.len(), |end| end + 3) + 1;
            continue;
        }
        if rest.starts_with("![cdata[") {
            cursor = start + rest.find("]]>").map_or(rest.len(), |end| end + 3) + 1;
            continue;
        }
        let tag_end = tag_end(&text[start..]).ok_or_else(|| bad("icon SVG has an unclosed tag"))?;
        let tag = &text[start + 1..start + tag_end];
        cursor = start + tag_end + 1;
        if tag.starts_with('?') || tag.starts_with('/') {
            continue;
        }
        let name_end = tag
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(tag.len());
        let name = tag[..name_end].to_ascii_lowercase();
        let name = name.rsplit(':').next().unwrap_or_default();
        if !root_seen {
            if name != "svg" {
                return Err(bad("icon SVG must start with an svg element"));
            }
            root_seen = true;
        }
        if FORBIDDEN_ELEMENTS.contains(&name) {
            return Err(bad("icon SVG must not contain active elements"));
        }
        check_attributes(&tag[name_end..])?;
    }
    if !root_seen {
        return Err(bad("icon SVG must start with an svg element"));
    }
    check_urls(&lower)
}

/// Index of the `>` closing the tag that starts at `text[0] == '<'`, skipping
/// quoted attribute values.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (index, c) in text.char_indices().skip(1) {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(index),
            (None, _) => {}
        }
    }
    None
}

fn check_attributes(mut rest: &str) -> Result<()> {
    let bad = || invalid("icon SVG has an event handler or an external reference");
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        if rest.is_empty() {
            return Ok(());
        }
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let value = match rest.strip_prefix('=') {
            None => "",
            Some(after) => {
                let after = after.trim_start();
                let (value, remaining) = match after.chars().next() {
                    Some(quote @ ('"' | '\'')) => {
                        let inner = &after[1..];
                        let end = inner.find(quote).ok_or_else(bad)?;
                        (&inner[..end], &inner[end + 1..])
                    }
                    _ => {
                        let end = after
                            .find(|c: char| c.is_ascii_whitespace())
                            .unwrap_or(after.len());
                        (&after[..end], &after[end..])
                    }
                };
                rest = remaining;
                value
            }
        };
        let local = name.rsplit(':').next().unwrap_or_default();
        if local.starts_with("on") {
            return Err(bad());
        }
        if matches!(local, "href" | "src") && !value.trim().starts_with('#') {
            return Err(bad());
        }
    }
}

/// Every `url(...)` must point at a fragment of the same document.
fn check_urls(lower: &str) -> Result<()> {
    let mut rest = lower;
    while let Some(at) = rest.find("url(") {
        let target = rest[at + 4..].trim_start_matches(|c: char| c.is_ascii_whitespace());
        let target = target.trim_start_matches(['"', '\'']);
        if !target.starts_with('#') {
            return Err(invalid("icon SVG refers to something outside itself"));
        }
        rest = &rest[at + 4..];
    }
    Ok(())
}
