//! JavaScript-compatible UTF-16 accounting used by the published JSON contract.
//!
//! TypeScript's `String.length`, `slice`, and match offsets count UTF-16 code
//! units, not Unicode scalar values or UTF-8 bytes. Rust strings cannot contain
//! lone surrogates, so slices that bisect a surrogate pair are decoded lossily;
//! all policy-generated boundaries normally land on scalar boundaries.

pub(crate) fn len(text: &str) -> usize {
    text.encode_utf16().count()
}

pub(crate) fn slice(text: &str, start: usize, end: usize) -> String {
    let units = text.encode_utf16().collect::<Vec<_>>();
    let start = start.min(units.len());
    let end = end.min(units.len()).max(start);
    String::from_utf16_lossy(&units[start..end])
}

pub(crate) fn find_from(text: &str, needle: &str, from: usize) -> Option<usize> {
    let units = text.encode_utf16().collect::<Vec<_>>();
    let needle_units = needle.encode_utf16().collect::<Vec<_>>();
    if needle_units.is_empty() || from > units.len() || needle_units.len() > units.len() {
        return None;
    }
    units[from..]
        .windows(needle_units.len())
        .position(|window| window == needle_units)
        .map(|offset| from + offset)
}

pub(crate) fn char_before(text: &str, offset: usize) -> Option<char> {
    let prefix = slice(text, 0, offset);
    prefix.chars().next_back()
}

pub(crate) fn char_at(text: &str, offset: usize) -> Option<char> {
    slice(text, offset, len(text)).chars().next()
}
