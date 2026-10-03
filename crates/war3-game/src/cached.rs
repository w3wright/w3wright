//! A memo over another asset source.
//!
//! # Why a wrapper rather than a field on the loader
//!
//! Every source in this workspace is behind [`AssetSource`], and reading a file from the game's
//! archives is the expensive operation: locate the member, decompress it, and for a `.slk` parse
//! hundreds of kilobytes of text. A lookup that happens per table row pays that per row unless
//! something remembers.
//!
//! Putting the memo here means it can be added to **any** source — the archives, a memory source,
//! a browser handle — without each of them growing its own cache, and without the loader having to
//! know whether it was given a cached source or a bare one.
//!
//! # What it does not do
//!
//! ⚠️ **No invalidation.** It remembers for as long as it lives, which is right for a game
//! installation — those files do not change under a running editor — and wrong for anything that
//! does. That is the reason this is a wrapper a caller opts into rather than something the loader
//! does invisibly: the caller knows what it is wrapping.
//!
//! Negative results are deliberately **not** cached. A miss in a layered source means every archive
//! and the loose tree were consulted, so caching it would be worth something, but a source that
//! gains a file later — a plugin loading asynchronously — would then be permanently wrong, and the
//! saving only matters for names, which are looked up repeatedly and found.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use war3_core::AssetSource;

/// A memo over another asset source.
///
/// `Arc<[u8]>` rather than `Vec<u8>` so that a caller who wants to keep the bytes seen here can
/// hold them without this cache holding a second copy.
#[derive(Debug)]
pub struct Cached<S: AssetSource> {
    inner: S,
    seen: Mutex<HashMap<String, Arc<[u8]>>>,
}

impl<S: AssetSource> Cached<S> {
    /// Wraps a source.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// The source underneath.
    #[must_use]
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// How many distinct files have been remembered.
    ///
    /// Exposed so a test can assert that a repeated lookup does not go back to the source, which is
    /// the whole claim this type makes.
    #[must_use]
    pub fn cached_count(&self) -> usize {
        self.seen.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Forgets everything. For a caller that knows the files underneath have changed.
    pub fn clear(&self) {
        if let Ok(mut map) = self.seen.lock() {
            map.clear();
        }
    }
}

impl<S: AssetSource> AssetSource for Cached<S> {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        // Look up first, and take the lock only to read. A poisoned lock falls through to the
        // source: a cache that cannot be read is a slow lookup, not a failure, and panicking here
        // would turn another thread's bug into this one's.
        if let Ok(map) = self.seen.lock() {
            if let Some(hit) = map.get(path) {
                return Some(hit.to_vec());
            }
        }

        let bytes = self.inner.get(path)?;

        if let Ok(mut map) = self.seen.lock() {
            map.insert(path.to_string(), Arc::from(bytes.as_slice()));
        }
        Some(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use war3_core::MemoryAssetSource;

    /// A source that counts how many times each file was actually read.
    #[derive(Debug)]
    struct Counting {
        inner: MemoryAssetSource,
        reads: AtomicUsize,
    }

    impl Counting {
        fn new(inner: MemoryAssetSource) -> Self {
            Self {
                inner,
                reads: AtomicUsize::new(0),
            }
        }
    }

    impl AssetSource for Counting {
        fn get(&self, path: &str) -> Option<Vec<u8>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.inner.get(path)
        }
    }

    /// The claim: asking twice reads once.
    #[test]
    fn a_repeated_lookup_reads_the_source_once() {
        let counting = Counting::new(MemoryAssetSource::new().with("a.txt", b"hello"));
        let cached = Cached::new(counting);

        assert_eq!(cached.get("a.txt").unwrap(), b"hello");
        assert_eq!(cached.get("a.txt").unwrap(), b"hello");
        assert_eq!(cached.get("a.txt").unwrap(), b"hello");

        assert_eq!(cached.inner().reads.load(Ordering::SeqCst), 1);
        assert_eq!(cached.cached_count(), 1);
    }

    /// A miss is not remembered, so a source that gains a file is not permanently wrong.
    #[test]
    fn a_miss_is_not_cached() {
        let counting = Counting::new(MemoryAssetSource::new());
        let cached = Cached::new(counting);

        assert!(cached.get("later.txt").is_none());
        assert!(cached.get("later.txt").is_none());

        assert_eq!(
            cached.inner().reads.load(Ordering::SeqCst),
            2,
            "a miss must go back to the source every time"
        );
        assert_eq!(cached.cached_count(), 0);
    }

    /// Two different paths are two different entries — the key is the path, not "the last file".
    #[test]
    fn paths_are_cached_separately() {
        let counting = Counting::new(
            MemoryAssetSource::new()
                .with("a.txt", b"a")
                .with("b.txt", b"b"),
        );
        let cached = Cached::new(counting);

        assert_eq!(cached.get("a.txt").unwrap(), b"a");
        assert_eq!(cached.get("b.txt").unwrap(), b"b");
        assert_eq!(cached.get("a.txt").unwrap(), b"a");
        assert_eq!(cached.cached_count(), 2);
        assert_eq!(cached.inner().reads.load(Ordering::SeqCst), 2);
    }

    /// `get_text` still works through the wrapper, since it is a default method on the trait.
    #[test]
    fn text_reads_go_through_the_cache_too() {
        let counting = Counting::new(MemoryAssetSource::new().with("a.txt", b"\xEF\xBB\xBFhello"));
        let cached = Cached::new(counting);

        assert_eq!(cached.get_text("a.txt").unwrap(), "hello");
        assert_eq!(cached.get_text("a.txt").unwrap(), "hello");
        assert_eq!(cached.inner().reads.load(Ordering::SeqCst), 1);
    }

    /// Clearing makes the next lookup go back to the source.
    #[test]
    fn clearing_forgets_everything() {
        let counting = Counting::new(MemoryAssetSource::new().with("a.txt", b"x"));
        let cached = Cached::new(counting);

        assert_eq!(cached.get("a.txt").unwrap(), b"x");
        cached.clear();
        assert_eq!(cached.cached_count(), 0);
        assert_eq!(cached.get("a.txt").unwrap(), b"x");
        assert_eq!(cached.inner().reads.load(Ordering::SeqCst), 2);
    }
}
