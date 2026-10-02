//! Little-endian writers shared by this crate's serialisers.
//!
//! Three formats (`war3map.w3i`, `war3map.doo`, `war3mapUnits.doo`) write the same
//! words, and each had grown its own copy of these functions. One copy, so a fix
//! — or a convention — lands everywhere at once.

use war3_core::Vec3;

/// `len` as the `i32` these formats use for count words.
pub(crate) fn len<T>(list: &[T]) -> i32 {
    i32::try_from(list.len()).unwrap_or(i32::MAX)
}

pub(crate) fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Three floats, in the order the formats store them.
pub(crate) fn push_vec3(out: &mut Vec<u8>, value: Vec3) {
    push_f32(out, value.x);
    push_f32(out, value.y);
    push_f32(out, value.z);
}
