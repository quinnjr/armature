//! Archive operations (ZIP)
//!
//! Provides functionality for creating and extracting ZIP archives.

use crate::{FileError, FileMetadata, FileResult, ProcessingResult};
use bytes::Bytes;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

/// Compression level for archives
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    /// No compression (store only)
    None,
    /// Fast compression
    Fast,
    /// Default compression
    Default,
    /// Best compression (slowest)
    Best,
}

impl CompressionLevel {
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

impl Default for CompressionLevel {
    fn default() -> Self {
        Self::Default
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
            zip.set_comment(comment.as_str());
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

    /// Build and save to file
    pub async fn save(self, path: impl AsRef<Path>) -> FileResult<ProcessingResult> {
        let result = self.build()?;
        result.save(path).await?;
        Ok(result)
    }
}

impl Default for ZipBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract a ZIP archive
pub struct ZipExtractor {
    data: Bytes,
}

impl ZipExtractor {
    /// Create a new ZIP extractor
    pub fn new(data: impl Into<Bytes>) -> Self {
        Self { data: data.into() }
    }

    /// List files in the archive
    pub fn list_files(&self) -> FileResult<Vec<String>> {
        let cursor = Cursor::new(&self.data);
        let archive = ZipArchive::new(cursor)
            .map_err(|e| FileError::Archive(format!("Failed to open archive: {}", e)))?;

        Ok(archive.file_names().map(|s| s.to_string()).collect())
    }

    /// Extract a single file by name
    pub fn extract_file(&self, name: &str) -> FileResult<Bytes> {
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| FileError::Archive(format!("Failed to open archive: {}", e)))?;

        let mut file = archive
            .by_name(name)
            .map_err(|e| FileError::Archive(format!("File not found: {}: {}", name, e)))?;

        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|e| FileError::Archive(format!("Failed to read {}: {}", name, e)))?;

        Ok(Bytes::from(data))
    }

    /// Extract all files
    pub fn extract_all(&self) -> FileResult<Vec<ArchiveEntry>> {
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| FileError::Archive(format!("Failed to open archive: {}", e)))?;

        let mut entries = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| FileError::Archive(format!("Failed to access file {}: {}", i, e)))?;

            if file.is_dir() {
                continue;
            }

            let name = file.name().to_string();
            let mut data = Vec::new();
            file.read_to_end(&mut data)
                .map_err(|e| FileError::Archive(format!("Failed to read {}: {}", name, e)))?;

            entries.push(ArchiveEntry::new(name, data));
        }

        Ok(entries)
    }

    /// Extract all files to a directory
    pub async fn extract_to(&self, dir: impl AsRef<Path>) -> FileResult<Vec<String>> {
        let dir = dir.as_ref();
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(FileError::Io)?;

        let entries = self.extract_all()?;
        let mut extracted = Vec::new();

        for entry in entries {
            let file_path = dir.join(&entry.path);

            // Create parent directories
            if let Some(parent) = file_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(FileError::Io)?;
            }

            tokio::fs::write(&file_path, &entry.data)
                .await
                .map_err(FileError::Io)?;
            extracted.push(entry.path);
        }

        Ok(extracted)
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
