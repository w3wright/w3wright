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

/// Name whose `FileKey` hash encrypts the hash table.
///
/// The tables are **not** keyed by an ASCII constant such as `HASH` or `BLK#`.
/// The format specification is explicit: "the hash table is encrypted using the
/// hash of `(hash table)` as the key", and likewise `(block table)` for the
/// block table, both with [`HashType::FileKey`].
pub const HASH_TABLE_KEY_NAME: &str = "(hash table)";

/// Name whose `FileKey` hash encrypts the block table.
pub const BLOCK_TABLE_KEY_NAME: &str = "(block table)";

/// Applies the format's word-wise cipher in place, recovering the plaintext.
///
/// # Why this is not simply "the inverse" of [`encrypt`]
///
/// The chain always advances on the **plaintext** word. Encryption has it in
/// hand because it is the input; decryption recovers it as `cipher ^ (key +
/// seed)`. Everything else — the table step, the key walk, the seed fold — is
/// identical, so the two functions differ in exactly one expression.
///
/// That single expression is why a generator must not be written by copying
/// this body and calling it "encrypt". Such a pair round-trips perfectly and
/// hides any mistake in the shared parts — which is how this crate came to
/// decrypt every real archive into noise while its own test archive read back
/// byte-identically. See the module tests for the pinned vectors that make that
/// failure impossible to reintroduce.
///
/// Used for the hash table, the block table and encrypted member contents. The
/// transformation is inherently sequential: both `seed` and `key` advance with
/// each word, so a slice cannot be processed in parallel or partially.
pub fn decrypt(table: &[u32; 0x500], data: &mut [u32], mut key: u32) {
    let mut seed: u32 = 0xEEEE_EEEE;
    for value in data.iter_mut() {
        seed = seed.wrapping_add(table[0x400 + (key & 0xFF) as usize]);
        let plain = *value ^ (key.wrapping_add(seed));
        // The key walks its own sequence and does **not** depend on the data.
        // Folding the plaintext into it here (a plausible-looking variant that
        // circulates widely, including in the pseudocode this crate was first
        // written from) makes every real archive decrypt into noise, while
        // still round-tripping archives this workspace generated itself.
        key = ((!key << 0x15).wrapping_add(0x1111_1111)) | (key >> 0x0B);
        seed = plain
            .wrapping_add(seed)
            .wrapping_add(seed << 5)
            .wrapping_add(3);
        *value = plain;
    }
}

/// Applies the format's word-wise cipher in place, turning plaintext into
/// ciphertext.
///
/// Identical to [`decrypt`] except that the seed fold consumes the word that
/// was *read* rather than the word that was *written* — which is the same
/// quantity, the plaintext, in both cases. The two are therefore exact
/// inverses: `decrypt(encrypt(words, k), k) == words`.
pub fn encrypt(table: &[u32; 0x500], data: &mut [u32], mut key: u32) {
    let mut seed: u32 = 0xEEEE_EEEE;
    for value in data.iter_mut() {
        seed = seed.wrapping_add(table[0x400 + (key & 0xFF) as usize]);
        let plain = *value;
        *value = plain ^ (key.wrapping_add(seed));
        key = ((!key << 0x15).wrapping_add(0x1111_1111)) | (key >> 0x0B);
        seed = plain
            .wrapping_add(seed)
            .wrapping_add(seed << 5)
            .wrapping_add(3);
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
    fn table_keys_are_the_hashes_of_their_names() {
        // The keys that encrypt the two tables are `HashString` of these
        // literal names with `MPQ_HASH_FILE_KEY`. They are *not* the ASCII
        // constants `HASH` and `BLK#`: using those makes every real archive's
        // tables decrypt into noise, with no error anywhere.
        //
        // Pinned because these two numbers decide whether any real file can be
        // read at all. `war3.mpq`'s hash table decrypts to 22084 empty slots out
        // of 32768 with the first, and to zero empty slots with anything else.
        let t = crypt_table();
        assert_eq!(
            hash_string(&t, HashType::FileKey, HASH_TABLE_KEY_NAME),
            0xC3AF_3770
        );
        assert_eq!(
            hash_string(&t, HashType::FileKey, BLOCK_TABLE_KEY_NAME),
            0xEC83_B3A3
        );
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
    fn encrypt_and_decrypt_are_exact_inverses_and_match_pinned_vectors() {
        // Pinned against an independent implementation of the specification's
        // `EncryptData`/`DecryptData`, and checked end to end against real
        // Blizzard archives (`war3.mpq`, `(4)LostTemple.w3m`, YDWE's sample).
        //
        // These numbers are the guard that the previous, wrong cipher could not
        // have passed: it folded the plaintext into the key update, and its own
        // generator shared the mistake, so nothing in this workspace noticed.
        let t = crypt_table();
        let plaintext: Vec<u32> = (1..=8u32).map(|i| i.wrapping_mul(0x1111_1111)).collect();
        let encrypted: Vec<u32> = [
            0x3A05_D103,
            0xB223_BDA6,
            0xA2AD_0871,
            0xC2A4_1977,
            0x941E_270C,
            0x3F8C_B1C0,
            0xB5C0_6688,
            0xD511_208D,
        ]
        .to_vec();

        let mut out = plaintext.clone();
        encrypt(&t, &mut out, 0x1234_5678);
        assert_eq!(out, encrypted, "encrypt must match the pinned ciphertext");

        let mut back = encrypted.clone();
        decrypt(&t, &mut back, 0x1234_5678);
        assert_eq!(back, plaintext, "decrypt must recover the pinned plaintext");
    }

    #[test]
    fn bytes_to_u32_ignores_trailing_partial_word() {
        assert_eq!(bytes_to_u32_le(&[1, 0, 0, 0, 2, 0, 0, 0, 9]), vec![1, 2]);
    }
}
