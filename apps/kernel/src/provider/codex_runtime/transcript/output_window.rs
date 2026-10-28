//! Keep a bounded, explicit tail in interactive tool transcript updates.
pub(super) const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const OMITTED: &str = "[earlier tool output omitted]\n";

pub(super) fn trim(text: &mut String) {
    if text.len() <= MAX_OUTPUT_BYTES {
        return;
    }
    let mut start = text.len() - (MAX_OUTPUT_BYTES - OMITTED.len());
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text.drain(..start);
    text.insert_str(0, OMITTED);
    // A single oversized notification must not leave its old allocation resident.
    if text.capacity() > MAX_OUTPUT_BYTES * 2 {
        text.shrink_to_fit();
    }
}

pub(super) fn bounded(text: &str) -> String {
    if text.len() <= MAX_OUTPUT_BYTES {
        return text.to_owned();
    }
    let mut start = text.len() - (MAX_OUTPUT_BYTES - OMITTED.len());
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("{OMITTED}{}", &text[start..])
}
