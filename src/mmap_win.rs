//! Windows-safe memory-mapped file wrapper.
//!
//! Provides [`MmapFile`], a cross-platform memory-mapped file reader with
//! explicit lifecycle management. On Windows the mapping view MUST be
//! released before the file handle is closed — otherwise the OS denies
//! rename/delete with `ERROR_SHARING_VIOLATION`.
//!
//! # Platform note
//!
//! Despite the module name this wrapper is cross-platform; the "win" suffix
//! indicates the design was driven by Windows semantics (mapping view
//! lifetime), which impose the strictest ordering requirements.
//!
//! # Empty files
//!
//! On some platforms `mmap` of length 0 is undefined behaviour. This
//! wrapper shortcuts the empty case: `open()` succeeds, `as_slice()` returns
//! an empty slice, and no actual mmap call is made.

use crate::error::TelemetryResult;
use memmap2::{Mmap, MmapOptions};
use std::fs::File;
use std::path::Path;

// ---------------------------------------------------------------------------
// MmapFile — memory-mapped file with explicit lifecycle
// ---------------------------------------------------------------------------

/// A memory-mapped file wrapper with explicit close semantics.
///
/// On Windows the mapping view holds a reference to the file handle.
/// To fully release the file (allowing rename/delete), the caller must
/// either call [`close`](Self::close) or let the struct drop — both
/// paths unmap first, then close the file handle.
///
/// # Empty files
///
/// For zero-length files no actual mapping is created. `as_slice()`
/// returns an empty slice and `close()` is a no-op.
///
/// # Safety
///
/// The `unsafe` mmap construction is fully encapsulated. The public API
/// is safe: `as_slice()` only returns bytes from the mapping that were
/// valid at `open()` time. If the underlying file is truncated or deleted
/// externally while mapped, Windows keeps the mapping alive (the pages
/// are reference-counted by the memory manager), so reads from
/// `as_slice()` will not segfault.
pub struct MmapFile {
        /// The file handle. Wrapped in `Option` so `close()` can take ownership
        /// and drop it in the correct order (after the mmap).
        ///
        /// Order in struct declaration matters: `mmap` is declared first so
        /// `Drop` releases it before `file`. This is critical on Windows.
        mmap: Option<Mmap>,
        file: Option<File>,
    }

impl MmapFile {
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    /// Open a file and memory-map it for reading.
    ///
    /// # Empty files
    ///
    /// If the file exists but has zero length, returns an `MmapFile` with
    /// no mapping. `as_slice()` returns `&[]`.
    ///
    /// # Errors
    ///
    /// Returns [`TelemetryResult::Err`] if the file cannot be opened or the
    /// mapping cannot be created (e.g. the file is locked exclusively).
    pub fn open(path: impl AsRef<Path>) -> TelemetryResult<Self> {
        let file = File::open(path.as_ref())?;
        let file_len = file.metadata()?.len() as usize;

        if file_len == 0 {
            return Ok(Self {
                mmap: None,
                file: Some(file),
            });
        }

        // SAFETY: The file is opened read-only and we do not modify it
        // (or allow external modification) while the mapping is live.
        // On Windows, the OS keeps mapped pages valid even if the file
        // is deleted externally — page-table entries are reference-counted
        // by the memory manager.
        let mmap = unsafe {
            MmapOptions::new()
                .len(file_len)
                .map(&file)
                .map_err(|e| {
                    // map_err because ? would still be inside unsafe{}
                    std::io::Error::new(e.kind(), e.to_string())
                })?
        };

        Ok(Self {
            mmap: Some(mmap),
            file: Some(file),
        })
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Return the entire mapped region as a byte slice.
    ///
    /// For empty files this returns an empty slice (`&[]`). The returned
    /// slice is valid for the lifetime of `&self`, i.e. until `close()` is
    /// called or the struct is dropped.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match &self.mmap {
            Some(mmap) => &mmap[..],
            None => &[],
        }
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Explicitly unmap the memory and close the file handle.
    ///
    /// Consumes `self`. After this call the file can be renamed or deleted
    /// without `ERROR_SHARING_VIOLATION` on Windows.
    ///
    /// # Order guarantee
    ///
    /// The mapping view is dropped **before** the file handle. This is the
    /// required order on Windows; reversing it would leave the file handle
    /// pinned by the mapping, preventing deletion.
    #[allow(dead_code)]
    pub fn close(mut self) -> TelemetryResult<()> {
        // 1. Unmap first (releases mapping view + duplicated handle).
        self.mmap = None;
        // 2. Then close the original file handle.
        self.file = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Drop — safety net when close() is not called
// ---------------------------------------------------------------------------

impl Drop for MmapFile {
    fn drop(&mut self) {
        // Order is guaranteed by Rust's struct drop order (fields dropped
        // in declaration order). `mmap` is declared before `file`, so it
        // is dropped first, releasing the mapping view before the file
        // handle is closed.
        //
        // We explicitly set to None anyway for clarity and to avoid any
        // confusion about which drop impl runs first.
        self.mmap = None;
        self.file = None;
    }
}

// ---------------------------------------------------------------------------
// Safety traits
// ---------------------------------------------------------------------------

// SAFETY: Mmap is Sync + Send on all supported platforms. The file handle
// is never accessed after construction (all reads go through the mmap).
unsafe impl Send for MmapFile {}
unsafe impl Sync for MmapFile {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    /// Helper: create a temp file with the given content, return the path.
    fn temp_file(name: &str, content: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mmap_win_test_{name}"));
        let mut file = File::create(&path).expect("create temp file");
        file.write_all(content).expect("write temp file");
        file.sync_all().expect("sync temp file");
        path
    }

    /// Helper: remove a temp file, ignoring errors if it's already gone.
    fn cleanup(path: &PathBuf) {
        let _ = std::fs::remove_file(path);
    }

    // -----------------------------------------------------------------------
    // Basic read tests
    // -----------------------------------------------------------------------

    #[test]
    fn map_known_file_reads_back_content() {
        let expected = b"Hello, mmap world! This is test content.\n";
        let path = temp_file("known_content", expected);
        let mf = MmapFile::open(&path).expect("open should succeed");
        assert_eq!(mf.as_slice(), expected);
        mf.close().expect("close should succeed");
        cleanup(&path);
    }

    #[test]
    fn empty_file_returns_empty_slice() {
        let path = temp_file("empty", b"");
        let mf = MmapFile::open(&path).expect("open should succeed for empty file");
        assert!(mf.as_slice().is_empty());
        mf.close().expect("close should succeed");
        cleanup(&path);
    }

    #[test]
    fn close_allows_file_deletion() {
        let path = temp_file("deletable", b"delete me");
        let mf = MmapFile::open(&path).expect("open should succeed");
        // Before close, deletion should fail (file is open).
        // Note: on some platforms like Linux, file deletion works even
        // while open. We primarily care about the post-close guarantee.
        mf.close().expect("close should succeed");

        // After close, the file should be deletable.
        std::fs::remove_file(&path).expect("should be able to delete after close");
    }

    #[test]
    fn open_nonexistent_file_returns_error() {
        let result = MmapFile::open("__nonexistent_test_file_12345.xyz");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Large file test
    // -----------------------------------------------------------------------

    #[test]
    fn large_file_random_access() {
        // Create a 10 MB file with known content at a specific offset.
        let file_size: usize = 10 * 1024 * 1024; // 10 MiB
        let check_offset: usize = 5 * 1024 * 1024; // 5 MiB
        let check_value: u8 = 0xAB;

        let path = temp_file("large_10mb", &[]);
        // Re-open for writing with specific content.
        {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("re-open temp file");
            file.set_len(file_size as u64).expect("set_len");
        }
        // Write a known byte at the check offset.
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("open for write");
            use std::io::Seek;
            file.seek(std::io::SeekFrom::Start(check_offset as u64))
                .expect("seek");
            file.write_all(&[check_value]).expect("write byte");
            file.sync_all().expect("sync");
        }

        let mf = MmapFile::open(&path).expect("open 10MB file");
        let slice = mf.as_slice();
        assert_eq!(slice.len(), file_size);
        assert_eq!(slice[check_offset], check_value);
        // Also spot-check: byte before should be 0 (set_len zero-fills on Windows).
        assert_eq!(slice[check_offset - 1], 0);
        // And byte after should be 0.
        assert_eq!(slice[check_offset + 1], 0);
        mf.close().expect("close 10MB file");
        cleanup(&path);
    }

    // -----------------------------------------------------------------------
    // Drop test
    // -----------------------------------------------------------------------

    #[test]
    fn drop_releases_file_handle() {
        let path = temp_file("drop_test", b"content for drop test");
        {
            let mf = MmapFile::open(&path).expect("open should succeed");
            assert_eq!(mf.as_slice(), b"content for drop test");
            // Let mf go out of scope — Drop should release handles.
        }
        // After drop, file should be deletable.
        std::fs::remove_file(&path).expect("should be able to delete after drop");
    }

    // -----------------------------------------------------------------------
    // External deletion test
    // -----------------------------------------------------------------------

    #[test]
    fn read_after_file_deleted_does_not_segfault() {
        let content = b"mapped content that outlives the file";
        let path = temp_file("outlive", content);
        let mf = MmapFile::open(&path).expect("open should succeed");

        // Delete the underlying file while mapped.
        std::fs::remove_file(&path).expect("delete while mapped");

        // On Windows, the memory manager keeps mapped pages alive until
        // all views are unmapped. Reading should not segfault.
        let slice = mf.as_slice();
        assert_eq!(slice, content);

        mf.close().expect("close should succeed");
        // File is already deleted, nothing to clean up.
    }

    // -----------------------------------------------------------------------
    // Edge case: reopen after close
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_after_close_works() {
        let content = b"reopen test";
        let path = temp_file("reopen", content);

        let mf1 = MmapFile::open(&path).expect("first open");
        assert_eq!(mf1.as_slice(), content);
        mf1.close().expect("first close");

        // Re-open the same file.
        let mf2 = MmapFile::open(&path).expect("second open");
        assert_eq!(mf2.as_slice(), content);
        mf2.close().expect("second close");

        cleanup(&path);
    }
}
