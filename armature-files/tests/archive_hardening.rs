//! Hardening regression tests for `armature_files::archive`.
//!
//! Covers the two standard archive attacks: path traversal ("Zip Slip") and
//! decompression bombs. Fixtures are built in-memory so these run anywhere.

#![cfg(feature = "archives")]

use armature_files::archive::{CompressionLevel, ZipBuilder, ZipExtractor};
use bytes::Bytes;
use std::io::Write;

/// Build a ZIP containing the given (name, contents) pairs *verbatim* — the
/// `zip` crate happily writes traversal and absolute member names, which is
/// exactly what a malicious archive looks like on the wire.
fn raw_zip(entries: &[(&str, &[u8])]) -> Bytes {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            writer
                .start_file(*name, options)
                .expect("zip writer should accept the raw member name");
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    Bytes::from(buffer.into_inner())
}

/// A traversal member (`../`) must be rejected outright, and nothing may be
/// written outside the extraction directory.
#[tokio::test]
async fn extract_to_rejects_parent_traversal_entries() {
    let archive = raw_zip(&[("../escaped.txt", b"pwned")]);

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("extract_here");
    let sibling = root.path().join("escaped.txt");

    let err = ZipExtractor::new(archive)
        .extract_to(&target)
        .await
        .expect_err("a `../` member must not be extracted");

    assert!(
        err.to_string().contains("escapes the extraction directory"),
        "unexpected error: {err}"
    );
    assert!(
        !sibling.exists(),
        "the traversal entry escaped to {}",
        sibling.display()
    );
}

/// An absolute member name must be neutralized: `Path::join` with an absolute
/// path discards the base entirely, so unguarded this writes straight to the
/// filesystem root. The entry must land *under* the extraction directory (or
/// be rejected) — never at the absolute location it names.
#[tokio::test]
async fn extract_to_neutralizes_absolute_entry_paths() {
    const ABSOLUTE: &str = "/tmp/armature-files-zip-slip-should-not-exist.txt";
    let archive = raw_zip(&[(ABSOLUTE, b"pwned")]);

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("out");

    let extracted = ZipExtractor::new(archive)
        .extract_to(&target)
        .await
        .expect("the entry should either be rejected or safely re-rooted");

    assert!(
        !std::path::Path::new(ABSOLUTE).exists(),
        "the absolute entry escaped the extraction directory"
    );

    for name in &extracted {
        let written = target.join(name);
        assert!(
            written
                .canonicalize()
                .unwrap()
                .starts_with(target.canonicalize().unwrap()),
            "{} was written outside the extraction directory",
            written.display()
        );
    }
}

/// The in-memory extraction path applies the same guard.
#[test]
fn extract_all_rejects_traversal_entries() {
    let archive = raw_zip(&[("a/../../b.txt", b"pwned")]);

    let err = ZipExtractor::new(archive)
        .extract_all()
        .expect_err("a traversal member must not be returned");

    assert!(
        err.to_string().contains("escapes the extraction directory"),
        "unexpected error: {err}"
    );
}

/// A high-ratio archive must be refused rather than decompressed into RAM: a
/// few KB of deflate stream expands to 4 MiB here, and unbounded that class of
/// input exhausts the heap.
#[test]
fn extract_all_enforces_the_uncompressed_size_budget() {
    let bomb = ZipBuilder::new()
        .compression(CompressionLevel::Best)
        .add_file("bomb.bin", vec![0u8; 4 * 1024 * 1024])
        .build()
        .unwrap();

    // Sanity check: the *compressed* archive is tiny, so a size check on the
    // input bytes would not catch this.
    assert!(
        bomb.data.len() < 64 * 1024,
        "fixture should be highly compressible, got {} bytes",
        bomb.data.len()
    );

    let err = ZipExtractor::new(bomb.data)
        .max_uncompressed_size(1024)
        .extract_all()
        .expect_err("a 4 MiB expansion must not fit in a 1 KiB budget");

    assert!(
        err.to_string().contains("budget"),
        "unexpected error: {err}"
    );
}

/// The budget is cumulative across entries, not per entry: each entry alone
/// fits, but together they blow the cap.
#[test]
fn uncompressed_size_budget_is_cumulative_across_entries() {
    let archive = ZipBuilder::new()
        .compression(CompressionLevel::Best)
        .add_file("one.bin", vec![7u8; 1024 * 1024])
        .add_file("two.bin", vec![9u8; 1024 * 1024])
        .build()
        .unwrap();

    // 1.5 MiB: the first entry fits, the second must not.
    let err = ZipExtractor::new(archive.data.clone())
        .max_uncompressed_size(3 * 1024 * 1024 / 2)
        .extract_all()
        .expect_err("the second entry should exhaust the shared budget");
    assert!(
        err.to_string().contains("budget"),
        "unexpected error: {err}"
    );

    // With headroom for both, extraction succeeds.
    let entries = ZipExtractor::new(archive.data)
        .max_uncompressed_size(4 * 1024 * 1024)
        .extract_all()
        .expect("both entries fit within a 4 MiB budget");
    assert_eq!(entries.len(), 2);
}

/// Entry-count is capped independently of total size.
#[test]
fn entry_count_is_capped() {
    let mut builder = ZipBuilder::new();
    for i in 0..20 {
        builder = builder.add_file(format!("f{i}.txt"), "x");
    }
    let archive = builder.build().unwrap();

    let err = ZipExtractor::new(archive.data)
        .max_entries(5)
        .list_files()
        .expect_err("20 entries must not pass a limit of 5");

    assert!(
        err.to_string().contains("exceeding the limit"),
        "unexpected error: {err}"
    );
}

/// A well-formed archive still round-trips through the on-disk path, and the
/// nested directory structure is recreated under the target directory.
#[tokio::test]
async fn extract_to_writes_nested_entries_under_the_target() {
    let archive = ZipBuilder::new()
        .add_file("top.txt", "top level")
        .add_file("nested/deep/file.txt", "nested content")
        .build()
        .unwrap();

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("out");

    let extracted = ZipExtractor::new(archive.data)
        .extract_to(&target)
        .await
        .expect("a well-formed archive should extract");

    assert_eq!(extracted.len(), 2);
    assert_eq!(
        std::fs::read_to_string(target.join("top.txt")).unwrap(),
        "top level"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("nested/deep/file.txt")).unwrap(),
        "nested content"
    );
}
