//! Archive operations (ZIP)
//!
//! Provides functionality for creating and extracting ZIP archives.

use crate::{FileError, FileMetadata, FileResult, ProcessingResult};
use bytes::Bytes;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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
    /// Parsed archive index, built lazily and reused.
    ///
    /// `ZipArchive::new` reads and indexes every entry, so re-parsing per call
    /// made the natural `list_files()` then `extract_file()` per name usage
    /// O(n^2) in entry count over attacker-controlled input.
    archive: Mutex<Option<ZipArchive<Cursor<Bytes>>>>,
}

impl ZipExtractor {
    /// Create a new ZIP extractor with the default resource limits.
    pub fn new(data: impl Into<Bytes>) -> Self {
        Self {
            data: data.into(),
            max_uncompressed_size: DEFAULT_MAX_UNCOMPRESSED_SIZE,
            max_entries: DEFAULT_MAX_ENTRIES,
            archive: Mutex::new(None),
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

    /// Open a fresh `ZipArchive` over the archive bytes.
    fn open(&self) -> FileResult<ZipArchive<Cursor<Bytes>>> {
        ZipArchive::new(Cursor::new(self.data.clone()))
            .map_err(|e| FileError::Archive(format!("Failed to open archive: {}", e)))
    }

    /// Run `f` against the cached (lazily parsed) archive index.
    fn with_archive<T>(
        &self,
        f: impl FnOnce(&mut ZipArchive<Cursor<Bytes>>) -> FileResult<T>,
    ) -> FileResult<T> {
        let mut guard = self
            .archive
            .lock()
            .map_err(|_| FileError::Archive("archive index lock poisoned".into()))?;

        if guard.is_none() {
            let archive = self.open()?;
            if archive.len() > self.max_entries {
                return Err(FileError::Archive(format!(
                    "archive contains {} entries, exceeding the limit of {}",
                    archive.len(),
                    self.max_entries
                )));
            }
            *guard = Some(archive);
        }

        f(guard.as_mut().expect("archive was just initialized"))
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
    pub fn extract_all(&self) -> FileResult<Vec<ArchiveEntry>> {
        let max_entries = self.max_entries;
        let mut budget = self.max_uncompressed_size;

        self.with_archive(move |archive| {
            let mut entries = Vec::new();

            for i in 0..archive.len() {
                if entries.len() >= max_entries {
                    return Err(FileError::Archive(format!(
                        "archive contains more than {max_entries} entries"
                    )));
                }

                let mut file = archive.by_index(i).map_err(|e| {
                    FileError::Archive(format!("Failed to access file {}: {}", i, e))
                })?;

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
        })
    }

    /// Extract all files to a directory.
    ///
    /// Each entry is streamed straight to disk under `dir` — the whole archive
    /// is never materialized in memory — and every resolved path is
    /// re-verified to live under the canonicalized `dir` before it is written,
    /// which also catches escapes through pre-existing symlinks.
    pub async fn extract_to(&self, dir: impl AsRef<Path>) -> FileResult<Vec<String>> {
        let dir = dir.as_ref().to_path_buf();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(FileError::Io)?;

        let root = tokio::fs::canonicalize(&dir).await.map_err(FileError::Io)?;

        let data = self.data.clone();
        let max_entries = self.max_entries;
        let max_uncompressed_size = self.max_uncompressed_size;

        // Inflating and writing every entry is blocking work; keep it off the
        // async runtime's worker threads.
        tokio::task::spawn_blocking(move || {
            let mut archive = ZipArchive::new(Cursor::new(data))
                .map_err(|e| FileError::Archive(format!("Failed to open archive: {}", e)))?;

            if archive.len() > max_entries {
                return Err(FileError::Archive(format!(
                    "archive contains {} entries, exceeding the limit of {}",
                    archive.len(),
                    max_entries
                )));
            }

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
                let file_path = root.join(&safe_path);

                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent).map_err(FileError::Io)?;
                    // Belt and braces: re-verify after the directories exist,
                    // so a symlink planted inside `dir` cannot redirect the
                    // write outside of it either.
                    let canonical_parent = parent.canonicalize().map_err(FileError::Io)?;
                    if !canonical_parent.starts_with(&root) {
                        return Err(FileError::Archive(format!(
                            "refusing to write {} outside the extraction directory",
                            file_path.display()
                        )));
                    }
                }

                let name = safe_path.to_string_lossy().to_string();
                let data = read_entry_within_budget(&mut file, &name, budget)?;
                budget -= data.len() as u64;

                std::fs::write(&file_path, &data).map_err(FileError::Io)?;
                extracted.push(name);
            }

            Ok(extracted)
        })
        .await
        .map_err(|e| FileError::Archive(format!("archive extraction task failed: {e}")))?
    }
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
