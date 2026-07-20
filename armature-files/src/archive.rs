//! Archive operations (ZIP)
//!
//! Provides functionality for creating and extracting ZIP archives.

use crate::{FileError, FileMetadata, FileResult, ProcessingResult};
use bytes::Bytes;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

/// Compression level for archives
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionLevel {
    /// No compression (store only)
    None,
    /// Fast compression
    Fast,
    /// Default compression
    #[default]
    Default,
    /// Best compression (slowest)
    Best,
}

impl CompressionLevel {
    #[allow(clippy::wrong_self_convention)]
    fn to_options(&self) -> SimpleFileOptions {
        match self {
            Self::None => {
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
            }
            Self::Fast => SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(1)),
            Self::Default => SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(6)),
            Self::Best => SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(9)),
        }
    }
}

/// A file to be added to an archive
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// Path within the archive
    pub path: String,
    /// File data
    pub data: Bytes,
}

impl ArchiveEntry {
    /// Create a new archive entry
    pub fn new(path: impl Into<String>, data: impl Into<Bytes>) -> Self {
        Self {
            path: path.into(),
            data: data.into(),
        }
    }
}

/// ZIP archive builder
pub struct ZipBuilder {
    entries: Vec<ArchiveEntry>,
    compression: CompressionLevel,
    comment: Option<String>,
}

impl ZipBuilder {
    /// Create a new ZIP builder
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            compression: CompressionLevel::Default,
            comment: None,
        }
    }

    /// Set compression level
    pub fn compression(mut self, level: CompressionLevel) -> Self {
        self.compression = level;
        self
    }

    /// Set archive comment
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Add a file to the archive
    pub fn add_file(mut self, path: impl Into<String>, data: impl Into<Bytes>) -> Self {
        self.entries.push(ArchiveEntry::new(path, data));
        self
    }

    /// Add multiple files
    pub fn add_files(mut self, entries: impl IntoIterator<Item = ArchiveEntry>) -> Self {
        self.entries.extend(entries);
        self
    }

    /// Add a directory of files from disk
    pub async fn add_directory(
        mut self,
        dir_path: impl AsRef<Path>,
        archive_prefix: &str,
    ) -> FileResult<Self> {
        let dir_path = dir_path.as_ref();

        let mut entries = Vec::new();
        let mut stack = vec![dir_path.to_path_buf()];

        while let Some(current) = stack.pop() {
            let mut dir = tokio::fs::read_dir(&current).await.map_err(FileError::Io)?;

            while let Some(entry) = dir.next_entry().await.map_err(FileError::Io)? {
                let path = entry.path();
                let file_type = entry.file_type().await.map_err(FileError::Io)?;

                if file_type.is_dir() {
                    stack.push(path);
                } else if file_type.is_file() {
                    let relative_path = path
                        .strip_prefix(dir_path)
                        .map_err(|e| FileError::Archive(e.to_string()))?;

                    let archive_path = if archive_prefix.is_empty() {
                        relative_path.to_string_lossy().to_string()
                    } else {
                        format!("{}/{}", archive_prefix, relative_path.to_string_lossy())
                    };

                    let data = tokio::fs::read(&path).await.map_err(FileError::Io)?;
                    entries.push(ArchiveEntry::new(archive_path, data));
                }
            }
        }

        self.entries.extend(entries);
        Ok(self)
    }

    /// Build the ZIP archive
    pub fn build(self) -> FileResult<ProcessingResult> {
        let start = std::time::Instant::now();

        let mut buffer = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(&mut buffer);

        let options = self.compression.to_options();

        for entry in &self.entries {
            // Normalize path separators
            let path = entry.path.replace('\\', "/");

            zip.start_file(&path, options)
                .map_err(|e| FileError::Archive(format!("Failed to add file {}: {}", path, e)))?;

            zip.write_all(&entry.data)
                .map_err(|e| FileError::Archive(format!("Failed to write {}: {}", path, e)))?;
        }

        if let Some(comment) = &self.comment {
            zip.set_comment(comment.as_str())
                .map_err(|e| FileError::Archive(format!("Failed to set comment: {}", e)))?;
        }

        zip.finish()
            .map_err(|e| FileError::Archive(format!("Failed to finalize archive: {}", e)))?;

        let data = Bytes::from(buffer.into_inner());

        Ok(ProcessingResult {
            data: data.clone(),
            metadata: FileMetadata {
                filename: "archive.zip".to_string(),
                mime_type: "application/zip".to_string(),
                size: data.len() as u64,
                extension: Some("zip".to_string()),
                width: None,
                height: None,
                pages: None,
            },
            operations: vec![format!("zip:create({} files)", self.entries.len())],
            processing_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Build and save to file.
    ///
    /// Compression is CPU-bound, so the build runs on a blocking thread
    /// instead of stalling a runtime worker.
    pub async fn save(self, path: impl AsRef<Path>) -> FileResult<ProcessingResult> {
        let result = self.build_async().await?;
        result.save(path).await?;
        Ok(result)
    }

    /// Build the archive on a blocking thread.
    ///
    /// Prefer this over [`Self::build`] from inside async code: deflating a
    /// large entry set blocks the calling runtime worker for its duration.
    pub async fn build_async(self) -> FileResult<ProcessingResult> {
        tokio::task::spawn_blocking(move || self.build())
            .await
            .map_err(|e| FileError::Archive(format!("archive build task failed: {e}")))?
    }
}

impl Default for ZipBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Default cap on the total uncompressed bytes a single extraction may
/// produce (256 MiB).
pub const DEFAULT_MAX_UNCOMPRESSED_SIZE: u64 = 256 * 1024 * 1024;

/// Default cap on the number of entries a single archive may contain.
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// Default cap on the size of the archive *container* itself, i.e. the
/// compressed bytes handed to [`ZipExtractor::new`] (64 MiB).
///
/// This is deliberately separate from [`DEFAULT_MAX_UNCOMPRESSED_SIZE`]: it
/// bounds the work done while merely *opening* the archive. `ZipArchive::new`
/// parses the whole central directory up front and allocates a record per
/// member, so the entry-count guard cannot fire until that allocation has
/// already happened.
pub const DEFAULT_MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// Read the member count out of the End Of Central Directory record without
/// parsing (and allocating) the central directory itself.
///
/// `ZipArchive::new` indexes every record before returning, so an entry-count
/// limit enforced afterwards is enforced too late: a ~46 MB archive of ~1M
/// minimal records exhausts memory before the guard can fire. The EOCD sits in
/// the last 22 + 65535 bytes and states the member count directly, so it can be
/// checked first, for the cost of a backwards scan.
///
/// Returns `None` when the record cannot be located or understood; callers must
/// treat that as "unknown" and fall back to the post-parse check.
fn peek_entry_count(data: &[u8]) -> Option<u64> {
    const EOCD_SIG: [u8; 4] = [b'P', b'K', 0x05, 0x06];
    const EOCD64_SIG: [u8; 4] = [b'P', b'K', 0x06, 0x06];

    // EOCD: 22 fixed bytes plus a comment of at most u16::MAX bytes.
    let window = data.len().min(22 + u16::MAX as usize);
    let tail = &data[data.len() - window..];

    let pos = tail.windows(4).rposition(|w| w == EOCD_SIG)?;
    let eocd = &tail[pos..];
    if eocd.len() < 22 {
        return None;
    }

    let count = u16::from_le_bytes([eocd[10], eocd[11]]);
    if count != u16::MAX {
        return Some(u64::from(count));
    }

    // The count is saturated, so the real value lives in the Zip64 EOCD record
    // (which precedes the Zip64 locator, which precedes the EOCD).
    let pos64 = tail[..pos].windows(4).rposition(|w| w == EOCD64_SIG)?;
    let eocd64 = &tail[pos64..];
    if eocd64.len() < 40 {
        return None;
    }
    Some(u64::from_le_bytes(eocd64[32..40].try_into().ok()?))
}

/// Resolve an archive member name to a path guaranteed to stay under the
/// extraction root.
///
/// Entry names come straight from the (untrusted) archive. `Path::join` with
/// an absolute member name discards the base entirely, and `../../.ssh/
/// authorized_keys` walks out of it — the classic "Zip Slip". The `zip`
/// crate's `enclosed_name` returns `None` for both, plus for names containing
/// NUL or invalid Windows drive prefixes.
fn safe_entry_path(file: &zip::read::ZipFile<'_, impl Read>) -> FileResult<PathBuf> {
    file.enclosed_name().ok_or_else(|| {
        FileError::Archive(format!(
            "refusing to extract entry with unsafe path {:?}: it escapes the extraction directory",
            file.name()
        ))
    })
}

/// Read at most `budget` bytes from `file`, erroring if it wants more.
///
/// Guards against zip bombs: a few-KB archive with a pathological compression
/// ratio otherwise decompresses into gigabytes of heap. The declared
/// uncompressed size is checked *before* reading, and the read is additionally
/// capped via `Read::take` so a lying header cannot get past the check.
fn read_entry_within_budget(
    file: &mut zip::read::ZipFile<'_, impl Read>,
    name: &str,
    budget: u64,
) -> FileResult<Vec<u8>> {
    if file.size() > budget {
        return Err(FileError::Archive(format!(
            "entry {name} declares {} uncompressed bytes, exceeding the remaining {budget} byte budget",
            file.size()
        )));
    }

    let mut data = Vec::new();
    // `budget + 1` so an over-long stream is detected rather than truncated.
    let read = std::io::copy(&mut file.by_ref().take(budget.saturating_add(1)), &mut data)
        .map_err(|e| FileError::Archive(format!("Failed to read {name}: {e}")))?;

    if read > budget {
        return Err(FileError::Archive(format!(
            "entry {name} expands beyond the {budget} byte budget (zip bomb?)"
        )));
    }

    Ok(data)
}

/// Extract a ZIP archive.
///
/// Extraction is hardened against the two standard archive attacks:
/// path traversal (see [`safe_entry_path`]) and decompression bombs (see
/// [`ZipExtractor::max_uncompressed_size`] / [`ZipExtractor::max_entries`]).
pub struct ZipExtractor {
    data: Bytes,
    max_uncompressed_size: u64,
    max_entries: usize,
    max_archive_bytes: u64,
    /// Parsed archive index, built lazily and reused.
    ///
    /// `ZipArchive::new` reads and indexes every entry, so re-parsing per call
    /// made the natural `list_files()` then `extract_file()` per name usage
    /// O(n^2) in entry count over attacker-controlled input.
    ///
    /// Behind an `Arc` so the blocking-thread entry points (`extract_to`,
    /// `extract_all_async`) can share the same cache instead of each paying a
    /// fresh central-directory parse.
    archive: Arc<Mutex<Option<ZipArchive<Cursor<Bytes>>>>>,
}

impl ZipExtractor {
    /// Create a new ZIP extractor with the default resource limits.
    pub fn new(data: impl Into<Bytes>) -> Self {
        Self {
            data: data.into(),
            max_uncompressed_size: DEFAULT_MAX_UNCOMPRESSED_SIZE,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_archive_bytes: DEFAULT_MAX_ARCHIVE_BYTES,
            archive: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the maximum total uncompressed size a single extraction may
    /// produce (default: [`DEFAULT_MAX_UNCOMPRESSED_SIZE`]).
    pub fn max_uncompressed_size(mut self, max_bytes: u64) -> Self {
        self.max_uncompressed_size = max_bytes;
        self
    }

    /// Set the maximum number of entries the archive may contain
    /// (default: [`DEFAULT_MAX_ENTRIES`]).
    pub fn max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries;
        self
    }

    /// Set the maximum size of the archive container itself, in compressed
    /// bytes (default: [`DEFAULT_MAX_ARCHIVE_BYTES`]).
    ///
    /// Checked *before* the archive is opened, because opening it is itself
    /// unbounded work proportional to the member count.
    pub fn max_archive_bytes(mut self, max_bytes: u64) -> Self {
        self.max_archive_bytes = max_bytes;
        self
    }

    /// Run `f` against the cached (lazily parsed) archive index.
    ///
    /// Free function rather than a method so the blocking-thread entry points
    /// can call it with a cloned `Arc` cache, which a `&self` method cannot
    /// hand to `spawn_blocking`.
    fn with_cached_archive<T>(
        cache: &Mutex<Option<ZipArchive<Cursor<Bytes>>>>,
        data: &Bytes,
        max_entries: usize,
        max_archive_bytes: u64,
        f: impl FnOnce(&mut ZipArchive<Cursor<Bytes>>) -> FileResult<T>,
    ) -> FileResult<T> {
        let mut guard = cache
            .lock()
            .map_err(|_| FileError::Archive("archive index lock poisoned".into()))?;

        if guard.is_none() {
            if data.len() as u64 > max_archive_bytes {
                return Err(FileError::Archive(format!(
                    "archive is {} bytes, exceeding the limit of {max_archive_bytes}",
                    data.len()
                )));
            }

            // Cheap pre-parse check: the central-directory index allocated by
            // `ZipArchive::new` is proportional to the member count, so the
            // entry limit has to be enforced before that, not after.
            if let Some(declared) = peek_entry_count(data)
                && declared > max_entries as u64
            {
                return Err(FileError::Archive(format!(
                    "archive declares {declared} entries, exceeding the limit of {max_entries}"
                )));
            }

            let archive = ZipArchive::new(Cursor::new(data.clone()))
                .map_err(|e| FileError::Archive(format!("Failed to open archive: {}", e)))?;
            if archive.len() > max_entries {
                return Err(FileError::Archive(format!(
                    "archive contains {} entries, exceeding the limit of {}",
                    archive.len(),
                    max_entries
                )));
            }
            *guard = Some(archive);
        }

        f(guard.as_mut().expect("archive was just initialized"))
    }

    /// Run `f` against the cached (lazily parsed) archive index.
    fn with_archive<T>(
        &self,
        f: impl FnOnce(&mut ZipArchive<Cursor<Bytes>>) -> FileResult<T>,
    ) -> FileResult<T> {
        Self::with_cached_archive(
            &self.archive,
            &self.data,
            self.max_entries,
            self.max_archive_bytes,
            f,
        )
    }

    /// List files in the archive
    pub fn list_files(&self) -> FileResult<Vec<String>> {
        self.with_archive(|archive| Ok(archive.file_names().map(|s| s.to_string()).collect()))
    }

    /// Extract a single file by name
    pub fn extract_file(&self, name: &str) -> FileResult<Bytes> {
        let budget = self.max_uncompressed_size;
        self.with_archive(|archive| {
            let mut file = archive
                .by_name(name)
                .map_err(|e| FileError::Archive(format!("File not found: {}: {}", name, e)))?;

            Ok(Bytes::from(read_entry_within_budget(
                &mut file, name, budget,
            )?))
        })
    }

    /// Extract all files into memory.
    ///
    /// Entries whose names escape the archive root are rejected, and the
    /// combined uncompressed size is capped. Prefer [`Self::extract_to`] for
    /// large archives: it streams each entry straight to disk instead of
    /// materializing every entry in RAM first.
    ///
    /// Prefer [`Self::extract_all_async`] from inside async code: inflating up
    /// to [`DEFAULT_MAX_UNCOMPRESSED_SIZE`] is CPU-bound work that blocks the
    /// calling runtime worker for its whole duration.
    pub fn extract_all(&self) -> FileResult<Vec<ArchiveEntry>> {
        let max_entries = self.max_entries;
        let budget = self.max_uncompressed_size;

        self.with_archive(move |archive| extract_all_from(archive, max_entries, budget))
    }

    /// Extract all files into memory on a blocking thread.
    ///
    /// Prefer this over [`Self::extract_all`] from inside async code.
    pub async fn extract_all_async(&self) -> FileResult<Vec<ArchiveEntry>> {
        let cache = Arc::clone(&self.archive);
        let data = self.data.clone();
        let max_entries = self.max_entries;
        let max_archive_bytes = self.max_archive_bytes;
        let budget = self.max_uncompressed_size;

        tokio::task::spawn_blocking(move || {
            Self::with_cached_archive(&cache, &data, max_entries, max_archive_bytes, |archive| {
                extract_all_from(archive, max_entries, budget)
            })
        })
        .await
        .map_err(|e| FileError::Archive(format!("archive extraction task failed: {e}")))?
    }

    /// Extract all files to a directory.
    ///
    /// Extraction is *staged*: every entry is written into a temporary
    /// directory created inside `dir`, and the results are only moved into
    /// place once the whole archive has been extracted successfully. Any
    /// failure — traversal, bomb budget, IO, a corrupt member — therefore
    /// leaves `dir` exactly as it was, instead of leaving behind however many
    /// entries had already been written (which let a crafted archive fill the
    /// caller's disk with attacker-controlled data and still return `Err`).
    ///
    /// Three separate guards keep writes inside the directory:
    ///
    /// 1. member names are resolved with [`safe_entry_path`], rejecting
    ///    absolute and `../` names outright;
    /// 2. each entry's parent directory is re-verified against the
    ///    canonicalized staging root after `create_dir_all`, catching escapes
    ///    through **directory** symlinks;
    /// 3. entries are created with `O_EXCL`, which refuses to follow a
    ///    pre-existing **file** symlink at the destination. A canonicalized
    ///    parent says nothing about the leaf: if `dir/config.yml` were already
    ///    a symlink to `/etc/app/config.yml`, its parent still canonicalizes
    ///    inside `dir` and a plain write would follow the link and clobber the
    ///    target.
    ///
    /// As a consequence extraction is **non-clobbering**: an entry whose
    /// destination name already exists in `dir` (as a file, directory or
    /// symlink) is an error rather than an overwrite, which is the right
    /// default for untrusted input.
    pub async fn extract_to(&self, dir: impl AsRef<Path>) -> FileResult<Vec<String>> {
        let dir = dir.as_ref().to_path_buf();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(FileError::Io)?;

        let root = tokio::fs::canonicalize(&dir).await.map_err(FileError::Io)?;

        let cache = Arc::clone(&self.archive);
        let data = self.data.clone();
        let max_entries = self.max_entries;
        let max_archive_bytes = self.max_archive_bytes;
        let max_uncompressed_size = self.max_uncompressed_size;

        // Inflating and writing every entry is blocking work; keep it off the
        // async runtime's worker threads.
        tokio::task::spawn_blocking(move || {
            Self::with_cached_archive(&cache, &data, max_entries, max_archive_bytes, |archive| {
                // Staged inside `root` so the promotion below is a same-
                // filesystem rename, and so `TempDir`'s `Drop` removes every
                // partially written entry on any early return.
                let staging = tempfile::tempdir_in(&root).map_err(FileError::Io)?;
                let staging_root = staging.path().canonicalize().map_err(FileError::Io)?;

                let mut budget = max_uncompressed_size;
                let mut extracted = Vec::new();

                for i in 0..archive.len() {
                    let mut file = archive.by_index(i).map_err(|e| {
                        FileError::Archive(format!("Failed to access file {}: {}", i, e))
                    })?;

                    if file.is_dir() {
                        continue;
                    }

                    let safe_path = safe_entry_path(&file)?;
                    let file_path = staging_root.join(&safe_path);

                    if let Some(parent) = file_path.parent() {
                        std::fs::create_dir_all(parent).map_err(FileError::Io)?;
                        let canonical_parent = parent.canonicalize().map_err(FileError::Io)?;
                        if !canonical_parent.starts_with(&staging_root) {
                            return Err(FileError::Archive(format!(
                                "refusing to write {} outside the extraction directory",
                                file_path.display()
                            )));
                        }
                    }

                    let name = safe_path.to_string_lossy().to_string();
                    let data = read_entry_within_budget(&mut file, &name, budget)?;
                    budget -= data.len() as u64;

                    // `create_new(true)` is `O_EXCL`: it fails rather than
                    // following a symlink that already sits at this path.
                    let mut handle = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&file_path)
                        .map_err(FileError::Io)?;
                    handle.write_all(&data).map_err(FileError::Io)?;

                    extracted.push((name, safe_path));
                }

                promote_staged_entries(&staging_root, &root, &extracted)?;

                Ok(extracted.into_iter().map(|(name, _)| name).collect())
            })
        })
        .await
        .map_err(|e| FileError::Archive(format!("archive extraction task failed: {e}")))?
    }
}

/// Extract every member of an already-opened archive into memory.
fn extract_all_from(
    archive: &mut ZipArchive<Cursor<Bytes>>,
    max_entries: usize,
    mut budget: u64,
) -> FileResult<Vec<ArchiveEntry>> {
    let mut entries = Vec::new();

    for i in 0..archive.len() {
        if entries.len() >= max_entries {
            return Err(FileError::Archive(format!(
                "archive contains more than {max_entries} entries"
            )));
        }

        let mut file = archive
            .by_index(i)
            .map_err(|e| FileError::Archive(format!("Failed to access file {}: {}", i, e)))?;

        if file.is_dir() {
            continue;
        }

        let safe_path = safe_entry_path(&file)?;
        let name = safe_path.to_string_lossy().to_string();
        let data = read_entry_within_budget(&mut file, &name, budget)?;
        budget -= data.len() as u64;

        entries.push(ArchiveEntry::new(name, data));
    }

    Ok(entries)
}

/// Move fully extracted entries from the staging directory into the real
/// extraction root.
///
/// Every destination is checked with `symlink_metadata` (which does *not*
/// follow symlinks) first: a pre-existing name is an error, so extraction never
/// clobbers, and never replaces something a symlink points at. All the checks
/// run before the first rename, so the usual failure mode (a name already
/// taken) still leaves the target directory untouched.
fn promote_staged_entries(
    staging_root: &Path,
    root: &Path,
    entries: &[(String, PathBuf)],
) -> FileResult<()> {
    for (name, relative) in entries {
        let destination = root.join(relative);
        if destination.symlink_metadata().is_ok() {
            return Err(FileError::Archive(format!(
                "refusing to overwrite the existing path {} with archive entry {name}",
                destination.display()
            )));
        }
    }

    for (_, relative) in entries {
        let destination = root.join(relative);

        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(FileError::Io)?;
            let canonical_parent = parent.canonicalize().map_err(FileError::Io)?;
            if !canonical_parent.starts_with(root) {
                return Err(FileError::Archive(format!(
                    "refusing to write {} outside the extraction directory",
                    destination.display()
                )));
            }
        }

        std::fs::rename(staging_root.join(relative), &destination).map_err(FileError::Io)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zip_builder_creation() {
        let builder = ZipBuilder::new()
            .compression(CompressionLevel::Best)
            .comment("Test archive");

        assert_eq!(builder.compression, CompressionLevel::Best);
        assert_eq!(builder.comment, Some("Test archive".to_string()));
    }

    #[test]
    fn test_zip_roundtrip() {
        let archive = ZipBuilder::new()
            .add_file("test.txt", "Hello, World!")
            .add_file("data/nested.txt", "Nested content")
            .build()
            .unwrap();

        let extractor = ZipExtractor::new(archive.data);
        let files = extractor.list_files().unwrap();

        assert!(files.contains(&"test.txt".to_string()));
        assert!(files.contains(&"data/nested.txt".to_string()));

        let content = extractor.extract_file("test.txt").unwrap();
        assert_eq!(&*content, b"Hello, World!");
    }
}
