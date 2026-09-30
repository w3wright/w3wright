//! MPQ hashing and encryption primitives.
//!
//! Two tables and three functions, all on wrapping 32-bit arithmetic.
//! Everything here must use `wrapping_*`: a debug build panics on overflow and
//! a release build would silently produce different values.

/// Which hash of a filename is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum HashType {
    /// Starting slot for the hash table probe.
    TableOffset = 0,
    /// Compared against the hash entry to confirm a slot holds the wanted file.
    NameA = 1,
    /// Used to derive the decryption key for a file's contents.
    NameB = 2,
    /// Used to derive the key for special files such as `(listfile)`.
    FileKey = 3,
}

impl HashType {
    /// Base offset of this hash's segment in the encryption table.
    #[inline]
    #[must_use]
    pub const fn table_offset(self) -> usize {
        (self as usize) * 0x100
    }
}

/// Builds the 0x500-entry encryption table.
///
/// The first 0x100 entries serve hashing (one segment per [`HashType`]), the
/// last 0x100 serve decryption.
///
/// # The generation rule
///
/// Each slot comes from **two modulo steps, taking the low 16 bits of each**:
///
/// ```text
/// seed  = (seed * 125 + 3) % 0x2AAAAB
/// high  = (seed & 0xFFFF) << 16
/// seed  = (seed * 125 + 3) % 0x2AAAAB
/// low   = seed & 0xFFFF
/// table = high | low
/// ```
///
/// Writing only `seed = seed * 125 + 3` (no modulus, no high/low packing)
/// yields a completely different table, and every archive then decrypts to
/// random bytes without any error being reported.
///
/// The table is a pure constant of its seed, so callers should generate it once
/// and reuse it rather than calling this on a hot path.
#[must_use]
pub fn crypt_table() -> [u32; 0x500] {
    const MODULUS: u32 = 0x002A_AAAB;
    let mut table = [0u32; 0x500];
    let mut seed: u32 = 0x0010_0001;
    for index in 0..0x100 {
        let mut i = index;
        for _ in 0..5 {
            seed = (seed.wrapping_mul(125).wrapping_add(3)) % MODULUS;
            let high = (seed & 0xFFFF) << 16;
            seed = (seed.wrapping_mul(125).wrapping_add(3)) % MODULUS;
            table[i] = high | (seed & 0xFFFF);
            i += 0x100;
        }
    }
    table
}

/// Hashes a filename.
///
/// Two things are easy to get wrong here, and both fail silently as
/// "that file does not exist":
///
/// 1. **The table index is the current character, not the hash's low byte.**
///    Some prose descriptions of the format write the lookup as
///    `table[(hash & 0xFF)]`, which is misleading. Using the hash value makes
///    *every* filename hash to the same number, so every lookup resolves to
///    whichever file landed in that slot first.
/// 2. Names are case-insensitive and `/` is equivalent to `\`.
#[must_use]
pub fn hash_string(table: &[u32; 0x500], hash_type: HashType, name: &str) -> u32 {
    let mut hash: u32 = 0x7FED_7FED;
    let mut seed: u32 = 0xEEEE_EEEE;
    for byte in name.bytes() {
        // ASCII uppercasing on purpose: these names are ASCII, and Unicode
        // uppercasing could change the byte length and therefore the hash.
        let upper = byte.to_ascii_uppercase();
        let ch = if upper == b'/' { b'\\' } else { upper };
        let idx = hash_type.table_offset() + usize::from(ch);
        hash = table[idx] ^ hash.wrapping_add(seed);
        seed = u32::from(ch)
            .wrapping_add(hash)
            .wrapping_add(seed)
            .wrapping_add(seed << 5)
            .wrapping_add(3);
    }
    hash
}

/// Decrypts 32-bit words in place.
///
/// Used for the hash table, the block table and encrypted file contents.
///
/// Decryption is inherently sequential: both `seed` and `key` advance with each
/// word, so a slice cannot be decrypted in parallel or partially.
pub fn decrypt(table: &[u32; 0x500], data: &mut [u32], mut key: u32) {
    let mut seed: u32 = 0xEEEE_EEEE;
    for value in data.iter_mut() {
        seed = seed.wrapping_add(table[0x400 + (key & 0xFF) as usize]);
        let plain = *value ^ (key.wrapping_add(seed));
        key = ((!key << 0x15).wrapping_add(3)) ^ plain.wrapping_add(seed).wrapping_add(seed << 5);
        seed = plain
            .wrapping_add(seed)
            .wrapping_add(seed << 5)
            .wrapping_add(3);
        *value = plain;
    }
}

/// Reinterprets bytes as little-endian `u32`s, ignoring a trailing partial word.
#[must_use]
pub fn bytes_to_u32_le(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypt_table_is_deterministic_and_matches_the_reference_algorithm() {
        let a = crypt_table();
        let b = crypt_table();
        assert_eq!(a, b);

        // Independent recomputation of the documented rule. This test exists to
        // catch the "LCG without the modulus" mistake, which makes every
        // archive decrypt into noise.
        const MODULUS: u32 = 0x002A_AAAB;
        let mut seed: u32 = 0x0010_0001;
        let mut expected = [0u32; 0x500];
        for index in 0..0x100usize {
            let mut i = index;
            for _ in 0..5 {
                seed = (seed.wrapping_mul(125).wrapping_add(3)) % MODULUS;
                let high = (seed & 0xFFFF) << 16;
                seed = (seed.wrapping_mul(125).wrapping_add(3)) % MODULUS;
                expected[i] = high | (seed & 0xFFFF);
                i += 0x100;
            }
        }
        assert_eq!(a, expected);
    }

    #[test]
    fn crypt_table_first_entries_are_stable() {
        // Pinned values, verified against an independent implementation. If the
        // generation rule changes, this fails immediately instead of showing up
        // later as "no files can be found in any archive".
        let t = crypt_table();
        assert_eq!(t[0x000], 0x55C6_36E2);
        assert_eq!(t[0x001], 0x02BE_0170);
        assert_eq!(t[0x0FF], 0x708C_9EEC);
        assert_eq!(t[0x100], 0x76F8_C1B1);
        assert_eq!(t[0x400], 0x193A_A698);
        assert_eq!(t[0x4FF], 0x7303_286C);
    }

    #[test]
    fn hash_uses_the_character_not_the_hash() {
        // Regression guard: using `hash & 0xFF` as the index makes all of these
        // collapse to one value. Pinned against an independent implementation.
        let t = crypt_table();
        assert_eq!(
            hash_string(&t, HashType::TableOffset, "WAR3MAP.W3I"),
            0x9871_45CE
        );
        assert_eq!(hash_string(&t, HashType::NameA, "WAR3MAP.W3I"), 0x33E8_87B7);
        assert_eq!(hash_string(&t, HashType::NameB, "WAR3MAP.W3I"), 0xD135_014A);
        assert_eq!(
            hash_string(&t, HashType::TableOffset, "WAR3MAP.W3E"),
            0xABD6_4F6D
        );
        assert_eq!(hash_string(&t, HashType::NameA, "WAR3MAP.W3E"), 0xF8C3_B168);
    }

    #[test]
    fn hash_is_case_and_separator_insensitive() {
        let t = crypt_table();
        let a = hash_string(&t, HashType::TableOffset, "war3map.w3i");
        let b = hash_string(&t, HashType::TableOffset, "WAR3MAP.W3I");
        let c = hash_string(&t, HashType::TableOffset, "War3Map.W3I");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn forward_slash_and_backslash_hash_the_same() {
        let t = crypt_table();
        let a = hash_string(&t, HashType::NameA, "UI\\TriggerData.txt");
        let b = hash_string(&t, HashType::NameA, "UI/TriggerData.txt");
        assert_eq!(a, b);
    }

    #[test]
    fn different_hash_types_give_different_values() {
        let t = crypt_table();
        let a = hash_string(&t, HashType::TableOffset, "(listfile)");
        let b = hash_string(&t, HashType::NameA, "(listfile)");
        let c = hash_string(&t, HashType::NameB, "(listfile)");
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn decrypt_is_deterministic() {
        let t = crypt_table();
        let original: Vec<u32> = (0..8u32).map(|i| i.wrapping_mul(0x9E37_79B9)).collect();

        let mut a = original.clone();
        decrypt(&t, &mut a, 0x1234_5678);
        let mut b = original.clone();
        decrypt(&t, &mut b, 0x1234_5678);
        assert_eq!(a, b);
        assert_ne!(a, original);
    }

    #[test]
    fn decrypt_key_chain_is_order_dependent() {
        // Same ciphertext, different order: the results must differ, proving the
        // key really does advance with position.
        let t = crypt_table();
        let mut a = vec![0xDEAD_BEEF, 0xCAFE_BABE];
        let mut b = vec![0xCAFE_BABE, 0xDEAD_BEEF];
        decrypt(&t, &mut a, 7);
        decrypt(&t, &mut b, 7);
        assert_ne!(a[0], b[1]);
    }

    #[test]
    fn bytes_to_u32_ignores_trailing_partial_word() {
        assert_eq!(bytes_to_u32_le(&[1, 0, 0, 0, 2, 0, 0, 0, 9]), vec![1, 2]);
    }
}
